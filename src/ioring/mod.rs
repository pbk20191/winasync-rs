//! I/O rings (`ioringapi.h`): batched, completion-queue file I/O.
//!
//! An I/O ring is two queues shared with the kernel. Operations are *built*
//! into the submission queue without a system call, handed over in one
//! `SubmitIoRing` however many there are, and their results are *popped*
//! from the completion queue, again without one. That batching is the whole
//! point: a hundred reads cost one submission instead of a hundred
//! `ReadFile`s.
//!
//! Files only -- Windows has no socket or pipe operations on a ring -- and
//! read, write and flush, which is what the kernel offers (read since
//! version 1, write and flush since version 3; this machine reports 400).
//!
//! # How it is driven
//!
//! An operation is built when its future is created and submitted when any
//! of them is first polled, so creating several before awaiting them (with
//! `join!`, say) submits them together. [`IoRing::submit`] does the same on
//! demand.
//!
//! Completions are popped by a pump: `SetIoRingCompletionEvent` signals an
//! event, a `TP_WAIT` on the process default pool waits for it, and the
//! callback drains the queue and wakes the futures -- the same arrangement
//! as `rio`'s queue. With `tp`, [`IoRing::with_runtime`] puts the pump on
//! that runtime's pool instead. [`IoRing::manual`] has no pump at all; the
//! caller calls [`IoRing::drive`].
//!
//! # Buffers
//!
//! Any [`IoBuf`] / [`IoBufMut`]: the safe operations take `'static` ones,
//! which move into the ring and come back with the result, so a dropped
//! future cancels its operation and the buffer simply stays in the ring
//! until the kernel is done with it. The `unsafe` `*_unchecked` operations
//! take borrowed ones; their futures, dropped early, cancel and then drain
//! the completion queue *on the dropping thread* until their own operation
//! has finished. That needs no pool thread, so it cannot deadlock the way a
//! wait for a callback could.
//!
//! # Registered files and buffers
//!
//! [`IoRing::register_files`] and [`IoRing::register_buffers`] (or
//! [`register_slab`](IoRing::register_slab)) give the kernel a table to name
//! files and memory by index, saving the handle lookup and the page probing
//! each operation would otherwise do. Nothing about the operations changes:
//! a [`RegisteredFile`] is a [`RingHandle`], a [`RingBuffer`] -- or a
//! `Slice` of one -- is an `IoBuf`, and `read` / `write` notice and use the
//! index.
//!
//! Measured, and built around:
//!
//! * the kernel keeps no reference to a registered handle, so the table
//!   keeps the handles (and the memory) itself;
//! * registering again *replaces* the whole table, and an old index would
//!   silently name the new table's entry -- so every index carries the
//!   registration it came from, and a replaced one is refused with
//!   `E_INVALIDARG`;
//! * a replacement takes effect only when its completion arrives, so
//!   operations that need a registration wait while one is pending; one
//!   that fails leaves no registration at all;
//! * an empty table is refused, so there is no unregistering -- replace, or
//!   drop the ring.
//!
//! # Capacity
//!
//! The completion queue is twice the submission queue, and no more
//! operations are ever in flight than it can hold -- an overflowing
//! completion queue would lose results. An operation created at that limit
//! waits for room before it is built.

use crate::common::pool::PoolRef;
use crate::common::buffer::{mark_read, IoBuf, IoBufMut};
use crate::common::tpctx::CtxHeader;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_HANDLE_EOF, GENERIC_READ, GENERIC_WRITE, HANDLE,
    IORING_E_SUBMISSION_QUEUE_FULL, S_FALSE,
};
use windows::Win32::Storage::FileSystem::{
    BuildIoRingCancelRequest, BuildIoRingFlushFile, BuildIoRingReadFile, BuildIoRingRegisterBuffers,
    BuildIoRingRegisterFileHandles, BuildIoRingWriteFile, IORING_BUFFER_INFO,
    CloseIoRing, CreateFileW, CreateIoRing, FILE_ATTRIBUTE_NORMAL, FILE_FLUSH_DEFAULT,
    FILE_SHARE_READ, FILE_WRITE_FLAGS_NONE, HIORING, IORING_BUFFER_REF, IORING_BUFFER_REF_0,
    IORING_CQE, IORING_CREATE_FLAGS, IORING_HANDLE_REF, IORING_HANDLE_REF_0, IORING_REF_RAW,
    IORING_REF_REGISTERED, IORING_REGISTERED_BUFFER,
    IORING_VERSION_3, IOSQE_FLAGS_NONE, CREATE_ALWAYS, OPEN_EXISTING, PopIoRingCompletion,
    SetIoRingCompletionEvent, SubmitIoRing,
};
use windows::Win32::System::Threading::{
    CallbackMayRunLong, CloseThreadpoolWait, CreateEventW, CreateThreadpoolWait,
    SetThreadpoolWait, WaitForThreadpoolWaitCallbacks, PTP_CALLBACK_INSTANCE, PTP_WAIT,
};
use windows_core::{Error, HRESULT, HSTRING, Result};

/// `UserData` of a cancel request: its completion carries nothing to deliver.
const CANCEL_TAG: usize = usize::MAX;

/// How long teardown keeps draining for operations it has cancelled before
/// giving up on them (and leaking their buffers rather than freeing memory
/// the kernel might still write).
const TEARDOWN_ROUNDS: u32 = 5000;

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// A file for ring operations. Cheap to clone; the handle closes with the
/// last clone *and* the last operation that uses it.
#[derive(Clone)]
pub struct File {
    handle: Arc<OwnedHandle>,
}

struct OwnedHandle(HANDLE);

// A kernel handle, usable from any thread.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl File {
    /// Opens an existing file for reading and writing.
    pub fn open(path: &str) -> Result<File> {
        Self::create_file(path, OPEN_EXISTING)
    }

    /// Creates a file, or truncates an existing one, for reading and writing.
    pub fn create(path: &str) -> Result<File> {
        Self::create_file(path, CREATE_ALWAYS)
    }

    fn create_file(
        path: &str,
        disposition: windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
    ) -> Result<File> {
        // Not FILE_FLAG_OVERLAPPED: a ring does not need it, and a handle
        // without it still works with every other API.
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ,
                None,
                disposition,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        };
        Ok(File {
            handle: Arc::new(OwnedHandle(handle)),
        })
    }

}

impl From<std::fs::File> for File {
    fn from(file: std::fs::File) -> Self {
        use std::os::windows::io::IntoRawHandle;
        File {
            handle: Arc::new(OwnedHandle(HANDLE(file.into_raw_handle()))),
        }
    }
}

impl From<std::os::windows::io::OwnedHandle> for File {
    fn from(handle: std::os::windows::io::OwnedHandle) -> Self {
        use std::os::windows::io::IntoRawHandle;
        File {
            handle: Arc::new(OwnedHandle(HANDLE(handle.into_raw_handle()))),
        }
    }
}

impl File {
    /// Takes ownership of `handle`: it is closed with the last clone and the
    /// last operation that uses it.
    ///
    /// # Safety
    /// `handle` must be an open file handle that nothing else will close.
    pub unsafe fn from_raw_handle(handle: HANDLE) -> File {
        File {
            handle: Arc::new(OwnedHandle(handle)),
        }
    }
}

/// A handle a ring operation can work on.
///
/// An operation keeps a clone of it until the kernel is done, which is how
/// a [`File`] or an `Arc<std::fs::File>` stays open for as long as the ring
/// needs it. The safe operations want `'static` handles; the
/// `*_unchecked` ones also take borrowed ones -- `&std::fs::File`, a
/// `BorrowedHandle` -- because their futures wait for the kernel when
/// dropped.
///
/// Any file handle works, opened with `FILE_FLAG_OVERLAPPED` or without,
/// except one bound to an I/O completion port. Measured: an operation on
/// such a handle completes at once with `E_INVALIDARG`, and nothing reaches
/// the port. So a `tp::File`, which its runtime's pool owns, is refused this
/// way rather than mis-delivered.
///
/// # Safety
/// [`raw_handle`](Self::raw_handle) must return a handle that stays open for
/// as long as the value it was called on exists -- the contract
/// `AsHandle` has.
pub unsafe trait RingHandle: Send + Sync {
    fn raw_handle(&self) -> HANDLE;

    /// Where this handle sits among a ring's registered files, if it is one
    /// ([`RegisteredFile`]); `None` for everything else.
    #[doc(hidden)]
    fn ring_registration(&self) -> Option<FileRegistration> {
        None
    }
}

/// A registered file's place in its ring; see [`RegisteredFile`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRegistration {
    pub ring: usize,
    pub generation: u64,
    pub index: u32,
}

unsafe impl RingHandle for File {
    fn raw_handle(&self) -> HANDLE {
        self.handle.0
    }
}

/// Any shared owner of a handle: `Arc<std::fs::File>`, `Arc<OwnedHandle>`.
unsafe impl<T: std::os::windows::io::AsHandle + Send + Sync> RingHandle for Arc<T> {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_handle().as_raw_handle())
    }
}

/// A borrow: `&std::fs::File`. With a `'static` borrow any operation takes
/// it; with a shorter one, only the `*_unchecked` operations.
unsafe impl<T: std::os::windows::io::AsHandle + Sync> RingHandle for &T {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_handle().as_raw_handle())
    }
}

unsafe impl RingHandle for std::os::windows::io::BorrowedHandle<'_> {
    fn raw_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.as_raw_handle())
    }
}

/// The reference an entry names `file` by: its registered index, or its raw
/// handle. Not validated -- [`BuildCtx::file_ref`] is what checks.
fn handle_ref<H: RingHandle>(file: &H) -> IORING_HANDLE_REF {
    match file.ring_registration() {
        Some(r) => IORING_HANDLE_REF {
            Kind: IORING_REF_REGISTERED,
            Handle: IORING_HANDLE_REF_0 { Index: r.index },
        },
        None => IORING_HANDLE_REF {
            Kind: IORING_REF_RAW,
            Handle: IORING_HANDLE_REF_0 {
                Handle: file.raw_handle(),
            },
        },
    }
}

// ---------------------------------------------------------------------------
// The ring
// ---------------------------------------------------------------------------

/// An I/O ring. Cheap to clone; every clone and every operation keeps it
/// open.
#[derive(Clone)]
pub struct IoRing {
    inner: Arc<Inner>,
}

struct Inner {
    ring: Mutex<Ring>,
    /// Signalled by the kernel when a completion is posted. Auto-reset: the
    /// pump's wait consumes it.
    event: HANDLE,
    /// The pump, if this ring has one. Taken by `Drop`.
    pump: Mutex<Option<Pump>>,
}

// The ring and event are kernel handles; everything else is behind the lock.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

struct Ring {
    handle: HIORING,
    slots: Vec<Slot>,
    free: Vec<u32>,
    /// Built since the last `SubmitIoRing`.
    unsubmitted: u32,
    /// Built and not yet popped -- cancel requests included, since their
    /// completions take room in the completion queue too.
    in_flight: u32,
    /// The completion queue's size: `in_flight` never exceeds it.
    capacity: u32,
    /// Operations waiting to be built: for `in_flight` to drop below
    /// `capacity`, or for a registration to be confirmed.
    room_waiters: Vec<Waker>,
    /// This ring's identity, which registered files and buffers carry so one
    /// ring's indexes cannot be used on another.
    id: usize,
    /// Every registration gets the next number; an index is valid only with
    /// the number of the registration that is active.
    next_generation: u64,
    files: Registration,
    buffers: Registration,
}

/// One kind of registration (files, or buffers) on a ring.
///
/// Registering again *replaces* the table (measured: index 0 then names the
/// new table's first entry, and the old indexes past its end are out of
/// range), so an index from an earlier registration would silently name
/// something else. Hence the generation. And a replacement is only known to
/// have taken when its completion arrives, so until then the registration
/// is `pending` and the operations that need it wait.
///
/// One that *fails* does not leave the old table usable either. Measured:
/// after the kernel refused an empty table, index 0 of the previous one was
/// "out of range" at build -- the ring's own record of the table had
/// already been replaced. So a failure leaves no registration at all, and
/// every index, old or new, is refused until the next one succeeds.
#[derive(Default)]
struct Registration {
    /// The active registration's number; 0 for none.
    generation: u64,
    /// Keeps the active table -- its handles, its memory -- alive while the
    /// kernel may use it.
    active: Option<Arc<dyn Send + Sync>>,
    pending: Option<PendingRegistration>,
}

struct PendingRegistration {
    user_data: usize,
    generation: u64,
    table: Arc<dyn Send + Sync>,
}

impl Registration {
    /// Whether an entry registered as `generation` on ring `owner` may be
    /// built on ring `ring` now.
    fn check(&self, ring: usize, owner: usize, generation: u64) -> core::result::Result<(), BuildStop> {
        if owner != ring {
            return Err(BuildStop::Fail(Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "registered on a different ring",
            )));
        }
        if self.pending.is_some() {
            return Err(BuildStop::Wait);
        }
        if generation != self.generation {
            return Err(BuildStop::Fail(Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "that registration has been replaced",
            )));
        }
        Ok(())
    }

    /// Settles a pending registration whose completion is `user_data`.
    /// Returns whether it was this registration's.
    fn settle(&mut self, user_data: usize, result: HRESULT) -> bool {
        match &self.pending {
            Some(p) if p.user_data == user_data => {
                let p = self.pending.take().unwrap();
                if result.is_ok() {
                    self.generation = p.generation;
                    self.active = Some(p.table);
                } else {
                    // Generations start at 1, so nothing matches 0.
                    self.generation = 0;
                    self.active = None;
                }
                true
            }
            _ => false,
        }
    }
}

/// Every ring's [`Ring::id`].
static NEXT_RING_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

struct Slot {
    generation: u32,
    state: SlotState,
}

enum SlotState {
    Free,
    Waiting {
        waker: Option<Waker>,
        kept: Kept,
        /// The future is gone; the result goes nowhere.
        abandoned: bool,
    },
    Done {
        result: HRESULT,
        information: usize,
        kept: Kept,
    },
}

/// An operation's payload -- its buffer and the file it reads -- held by
/// the ring for as long as the kernel may use it. Type-erased so one slot
/// type serves every operation, and not limited to `'static` payloads so
/// the `*_unchecked` operations can use it too.
struct Kept {
    ptr: *mut (),
    drop: unsafe fn(*mut ()),
}

// Constructed only from `Send` payloads.
unsafe impl Send for Kept {}

impl Kept {
    fn new<P: Send>(payload: P) -> Self {
        unsafe fn drop_box<P>(ptr: *mut ()) {
            drop(unsafe { Box::from_raw(ptr.cast::<P>()) });
        }
        Kept {
            ptr: Box::into_raw(Box::new(payload)).cast(),
            drop: drop_box::<P>,
        }
    }

    /// # Safety
    /// `P` must be the type this was made from.
    unsafe fn take<P>(self) -> P {
        let payload = *unsafe { Box::from_raw(self.ptr.cast::<P>()) };
        std::mem::forget(self);
        payload
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.ptr) }
    }
}

fn pack(index: u32, generation: u32) -> usize {
    (generation as usize) << 32 | index as usize
}

fn unpack(user_data: usize) -> (u32, u32) {
    (user_data as u32, (user_data >> 32) as u32)
}

impl IoRing {
    /// A ring with room for `entries` operations in each submission, pumped
    /// from the process default thread pool.
    pub fn new(entries: u32) -> Result<IoRing> {
        Self::build(entries, Some(PoolRef::ProcessDefault))
    }

    /// [`new`](Self::new), with the pump on `rt`'s pool.
    #[cfg(feature = "tp")]
    pub fn with_runtime(rt: &crate::tp::Runtime, entries: u32) -> Result<IoRing> {
        Self::build(entries, Some(rt.inner.pool_ref()))
    }

    /// A ring with no pump: completions are delivered only when the caller
    /// calls [`drive`](Self::drive).
    pub fn manual(entries: u32) -> Result<IoRing> {
        Self::build(entries, None)
    }

    /// `pump` is the pool to pump from; `None` for a manual ring.
    fn build(entries: u32, pump: Option<PoolRef>) -> Result<IoRing> {
        let entries = entries.max(1);
        let capacity = entries.saturating_mul(2);
        let handle = unsafe {
            CreateIoRing(IORING_VERSION_3, IORING_CREATE_FLAGS::default(), entries, capacity)?
        };
        let event = match unsafe { CreateEventW(None, false, false, None) } {
            Ok(event) => event,
            Err(e) => {
                unsafe {
                    let _ = CloseIoRing(handle);
                }
                return Err(e);
            }
        };
        let close_both = |e: Error| {
            unsafe {
                let _ = CloseIoRing(handle);
                let _ = CloseHandle(event);
            }
            e
        };
        unsafe { SetIoRingCompletionEvent(handle, event) }.map_err(close_both)?;

        let inner = Arc::new(Inner {
            ring: Mutex::new(Ring {
                handle,
                slots: Vec::new(),
                free: Vec::new(),
                unsubmitted: 0,
                in_flight: 0,
                capacity,
                room_waiters: Vec::new(),
                id: NEXT_RING_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                next_generation: 1,
                files: Registration::default(),
                buffers: Registration::default(),
            }),
            event,
            pump: Mutex::new(None),
        });
        if let Some(pool) = pump {
            let pump = Pump::start(&inner, pool)?;
            *inner.pump.lock().unwrap() = Some(pump);
        }
        Ok(IoRing { inner })
    }

    /// Submits every operation built so far; returns how many went.
    pub fn submit(&self) -> Result<u32> {
        submit_locked(&mut self.inner.ring.lock().unwrap())
    }

    /// Pops and delivers every completion waiting in the queue; returns how
    /// many. The only way results arrive on a [`manual`](Self::manual) ring,
    /// and harmless on a pumped one.
    pub fn drive(&self) -> usize {
        let (n, wake, dead) = drain_locked(&mut self.inner.ring.lock().unwrap());
        wake.into_iter().for_each(Waker::wake);
        drop(dead);
        n
    }

    /// Reads into `buf` at `offset`; resolves to the bytes read (0 at end of
    /// file) and hands the buffer back.
    pub fn read<H: RingHandle + Clone + 'static, B: IoBufMut + 'static>(
        &self,
        file: &H,
        buf: B,
        offset: u64,
    ) -> RingOp<ReadOp<H, B>> {
        RingOp::new(&self.inner, ReadOp { file: file.clone(), buf, offset }, false)
    }

    /// Writes `buf` at `offset`; resolves to the bytes written and hands the
    /// buffer back.
    pub fn write<H: RingHandle + Clone + 'static, B: IoBuf + 'static>(
        &self,
        file: &H,
        buf: B,
        offset: u64,
    ) -> RingOp<WriteOp<H, B>> {
        RingOp::new(&self.inner, WriteOp { file: file.clone(), buf, offset }, false)
    }

    /// Flushes `file`'s buffers to the device.
    pub fn flush<H: RingHandle + Clone + 'static>(&self, file: &H) -> RingOp<FlushOp<H>> {
        RingOp::new(&self.inner, FlushOp { file: file.clone() }, false)
    }

    /// [`read`](Self::read), with a buffer that need not be `'static`.
    ///
    /// Dropped before it completes, the future cancels the read and then
    /// pops completions itself until the read's own has arrived, so the
    /// borrow cannot end while the kernel still writes into it. No pool
    /// thread is involved.
    ///
    /// # Safety
    /// The future must be dropped or run to completion -- never forgotten or
    /// leaked (`mem::forget`, a reference cycle, `Box::leak`, an executor
    /// that frees tasks without dropping them). One that disappears without
    /// its `Drop` running leaves the kernel writing into memory the borrow
    /// has already given back.
    pub unsafe fn read_unchecked<H: RingHandle + Clone, B: IoBufMut>(
        &self,
        file: &H,
        buf: B,
        offset: u64,
    ) -> RingOp<ReadOp<H, B>> {
        RingOp::new(&self.inner, ReadOp { file: file.clone(), buf, offset }, true)
    }

    /// [`write`](Self::write), with a buffer that need not be `'static`; see
    /// [`read_unchecked`](Self::read_unchecked).
    ///
    /// # Safety
    /// As for [`read_unchecked`](Self::read_unchecked): the future must be
    /// dropped or run to completion, never forgotten or leaked.
    pub unsafe fn write_unchecked<H: RingHandle + Clone, B: IoBuf>(
        &self,
        file: &H,
        buf: B,
        offset: u64,
    ) -> RingOp<WriteOp<H, B>> {
        RingOp::new(&self.inner, WriteOp { file: file.clone(), buf, offset }, true)
    }
}

/// `SubmitIoRing` without waiting. A ring with nothing built is a no-op.
fn submit_locked(ring: &mut Ring) -> Result<u32> {
    if ring.unsubmitted == 0 {
        return Ok(0);
    }
    let mut submitted = 0u32;
    unsafe { SubmitIoRing(ring.handle, 0, 0, Some(&mut submitted))? };
    ring.unsubmitted = ring.unsubmitted.saturating_sub(submitted);
    Ok(submitted)
}

/// Pops every completion and settles its slot. Returns the count, the
/// wakers to wake and the abandoned payloads to drop -- both *after* the
/// lock is released, since either may run arbitrary code.
fn drain_locked(ring: &mut Ring) -> (usize, Vec<Waker>, Vec<Kept>) {
    let mut wake = Vec::new();
    let mut dead = Vec::new();
    let mut n = 0;
    loop {
        let mut cqe = IORING_CQE::default();
        let hr = unsafe { PopIoRingCompletion(ring.handle, &mut cqe) };
        if hr == S_FALSE || hr.is_err() {
            break;
        }
        n += 1;
        ring.in_flight = ring.in_flight.saturating_sub(1);
        if cqe.UserData == CANCEL_TAG {
            continue;
        }
        // A registration's completion confirms (or refuses) it before its
        // future hears; either way the operations waiting on it may build.
        if ring.files.settle(cqe.UserData, cqe.ResultCode)
            | ring.buffers.settle(cqe.UserData, cqe.ResultCode)
        {
            wake.append(&mut ring.room_waiters);
        }
        let (index, generation) = unpack(cqe.UserData);
        let Some(slot) = ring.slots.get_mut(index as usize) else {
            continue;
        };
        if slot.generation != generation {
            continue;
        }
        match std::mem::replace(&mut slot.state, SlotState::Free) {
            SlotState::Waiting {
                abandoned: true,
                kept,
                ..
            } => {
                dead.push(kept);
                ring.free.push(index);
                slot.generation = slot.generation.wrapping_add(1);
            }
            SlotState::Waiting { waker, kept, .. } => {
                slot.state = SlotState::Done {
                    result: cqe.ResultCode,
                    information: cqe.Information,
                    kept,
                };
                wake.extend(waker);
            }
            other => slot.state = other,
        }
    }
    if ring.in_flight < ring.capacity {
        wake.append(&mut ring.room_waiters);
    }
    (n, wake, dead)
}

impl Ring {
    fn alloc(&mut self, kept: Kept) -> (u32, u32) {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    state: SlotState::Free,
                });
                self.slots.len() as u32 - 1
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.state = SlotState::Waiting {
            waker: None,
            kept,
            abandoned: false,
        };
        (index, slot.generation)
    }

    fn release(&mut self, index: u32) {
        let slot = &mut self.slots[index as usize];
        slot.state = SlotState::Free;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
    }

    /// Asks the kernel to cancel the operation `user_data` names on `file`.
    /// Skipped when the completion queue has no room for the cancel's own
    /// completion: the operation then simply runs to its end.
    fn cancel(&mut self, file: IORING_HANDLE_REF, user_data: usize) {
        if self.in_flight >= self.capacity {
            return;
        }
        let built = build_with_retry(self, |h| unsafe {
            BuildIoRingCancelRequest(h, file, user_data, CANCEL_TAG)
        });
        if built.is_ok() {
            self.in_flight += 1;
            self.unsubmitted += 1;
            let _ = submit_locked(self);
        }
    }
}

/// Builds one entry, submitting once and retrying if the submission queue is
/// full.
fn build_with_retry(ring: &mut Ring, build: impl Fn(HIORING) -> Result<()>) -> Result<()> {
    match build(ring.handle) {
        Err(e) if e.code() == IORING_E_SUBMISSION_QUEUE_FULL => {
            submit_locked(ring)?;
            build(ring.handle)
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Building entries
// ---------------------------------------------------------------------------

/// Why an operation was not built.
#[doc(hidden)]
pub enum BuildStop {
    /// Not yet: a registration it uses is still being confirmed.
    Wait,
    /// Never: resolve the future with this error.
    Fail(Error),
}

impl From<Error> for BuildStop {
    fn from(e: Error) -> Self {
        BuildStop::Fail(e)
    }
}

/// What an operation builds its entry with: the ring, under its lock, and
/// the `UserData` its completion will carry.
#[doc(hidden)]
pub struct BuildCtx<'a> {
    ring: &'a mut Ring,
    user_data: usize,
}

impl BuildCtx<'_> {
    /// How the entry names `file`, checked against the ring's registered
    /// files if it is one of them.
    fn file_ref<H: RingHandle>(&self, file: &H) -> core::result::Result<IORING_HANDLE_REF, BuildStop> {
        if let Some(r) = file.ring_registration() {
            self.ring.files.check(self.ring.id, r.ring, r.generation)?;
        }
        Ok(handle_ref(file))
    }

    /// How the entry names `len` bytes of `buf` starting at `ptr`: by
    /// registered index and offset if `buf` is registered memory of this
    /// ring, else by address.
    fn buffer_ref<B: IoBuf + ?Sized>(
        &self,
        buf: &B,
        ptr: *mut core::ffi::c_void,
    ) -> core::result::Result<IORING_BUFFER_REF, BuildStop> {
        match buf.ring_registration() {
            Some(r) => {
                self.ring.buffers.check(self.ring.id, r.ring, r.generation)?;
                Ok(IORING_BUFFER_REF {
                    Kind: IORING_REF_REGISTERED,
                    Buffer: IORING_BUFFER_REF_0 {
                        IndexAndOffset: IORING_REGISTERED_BUFFER {
                            BufferIndex: r.index,
                            Offset: r.offset,
                        },
                    },
                })
            }
            None => Ok(IORING_BUFFER_REF {
                Kind: IORING_REF_RAW,
                Buffer: IORING_BUFFER_REF_0 { Address: ptr },
            }),
        }
    }

    /// Builds the entry, submitting once and retrying if the submission
    /// queue is full.
    fn entry(&mut self, build: impl Fn(HIORING, usize) -> Result<()>) -> core::result::Result<(), BuildStop> {
        let user_data = self.user_data;
        build_with_retry(self.ring, |h| build(h, user_data))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// What a ring operation knows how to do: build its entry, and turn the
/// completion back into a result.
pub trait RingOperation: Send + Sized {
    /// What the future resolves to.
    type Output;

    #[doc(hidden)]
    fn file(&self) -> IORING_HANDLE_REF;

    /// Builds the submission entry. `self` is at its final address: the
    /// payload has been boxed into the ring already.
    #[doc(hidden)]
    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop>;

    /// The result: `information` is the completion's `Information` field.
    #[doc(hidden)]
    fn finish(self, result: HRESULT, information: usize) -> Self::Output;

    /// The result when the operation could not be built at all.
    #[doc(hidden)]
    fn fail(self, error: Error) -> Self::Output;
}

/// A read: see [`IoRing::read`].
pub struct ReadOp<H, B> {
    file: H,
    buf: B,
    offset: u64,
}

impl<H: RingHandle, B: IoBufMut> RingOperation for ReadOp<H, B> {
    type Output = (Result<usize>, B);

    fn file(&self) -> IORING_HANDLE_REF {
        handle_ref(&self.file)
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        let len = self.buf.bytes_total() as u32;
        let file = ctx.file_ref(&self.file)?;
        let ptr = self.buf.stable_mut_ptr().cast();
        let data = ctx.buffer_ref(&self.buf, ptr)?;
        let offset = self.offset;
        ctx.entry(|h, user_data| unsafe {
            BuildIoRingReadFile(h, file, data, len, offset, user_data, IOSQE_FLAGS_NONE)
        })
    }

    fn finish(self, result: HRESULT, information: usize) -> Self::Output {
        let result = if result == ERROR_HANDLE_EOF.to_hresult() {
            Ok(0)
        } else {
            result.ok().map(|()| information)
        };
        let n = result.as_ref().ok().copied();
        (result, mark_read(n, self.buf))
    }

    fn fail(self, error: Error) -> Self::Output {
        (Err(error), self.buf)
    }
}

/// A write: see [`IoRing::write`].
pub struct WriteOp<H, B> {
    file: H,
    buf: B,
    offset: u64,
}

impl<H: RingHandle, B: IoBuf> RingOperation for WriteOp<H, B> {
    type Output = (Result<usize>, B);

    fn file(&self) -> IORING_HANDLE_REF {
        handle_ref(&self.file)
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        let len = IoBuf::len(&self.buf) as u32;
        let file = ctx.file_ref(&self.file)?;
        // One pointer type for both directions; a write only reads it.
        let ptr = self.buf.stable_ptr() as *mut core::ffi::c_void;
        let data = ctx.buffer_ref(&self.buf, ptr)?;
        let offset = self.offset;
        ctx.entry(|h, user_data| unsafe {
            BuildIoRingWriteFile(h, file, data, len, offset, FILE_WRITE_FLAGS_NONE, user_data, IOSQE_FLAGS_NONE)
        })
    }

    fn finish(self, result: HRESULT, information: usize) -> Self::Output {
        (result.ok().map(|()| information), self.buf)
    }

    fn fail(self, error: Error) -> Self::Output {
        (Err(error), self.buf)
    }
}

/// A flush: see [`IoRing::flush`].
pub struct FlushOp<H> {
    file: H,
}

impl<H: RingHandle> RingOperation for FlushOp<H> {
    type Output = Result<()>;

    fn file(&self) -> IORING_HANDLE_REF {
        handle_ref(&self.file)
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        let file = ctx.file_ref(&self.file)?;
        ctx.entry(|h, user_data| unsafe {
            BuildIoRingFlushFile(h, file, FILE_FLUSH_DEFAULT, user_data, IOSQE_FLAGS_NONE)
        })
    }

    fn finish(self, result: HRESULT, _information: usize) -> Self::Output {
        result.ok()
    }

    fn fail(self, error: Error) -> Self::Output {
        Err(error)
    }
}

/// One operation on an [`IoRing`], as a future.
///
/// Built when created (so operations created together are submitted
/// together), submitted when first polled. Dropping it before it completes
/// cancels the operation; see [`IoRing`] for what happens to the buffer.
pub struct RingOp<O: RingOperation> {
    inner: Arc<Inner>,
    stage: Stage<O>,
    /// Set for the `*_unchecked` operations: a drop waits for the kernel.
    wait_on_drop: bool,
}

enum Stage<O> {
    /// Not built yet: the completion queue was full, or a registration it
    /// uses was still being confirmed.
    Unbuilt(O),
    /// In the ring, slot `index` of generation `generation`.
    Built {
        index: u32,
        generation: u32,
        file: IORING_HANDLE_REF,
    },
    /// Could not be built; resolves to this error.
    Failed(O, Error),
    Finished,
}

// The handle reference is a plain handle value; the payload is `Send`.
unsafe impl<O: RingOperation> Send for RingOp<O> {}
impl<O: RingOperation> Unpin for RingOp<O> {}

impl<O: RingOperation> RingOp<O> {
    /// An operation that resolves to `error` without being built.
    fn failed(inner: &Arc<Inner>, op: O, error: Error) -> Self {
        RingOp {
            inner: inner.clone(),
            stage: Stage::Failed(op, error),
            wait_on_drop: false,
        }
    }

    fn new(inner: &Arc<Inner>, op: O, wait_on_drop: bool) -> Self {
        let mut this = RingOp {
            inner: inner.clone(),
            stage: Stage::Unbuilt(op),
            wait_on_drop,
        };
        this.try_build(&mut inner.ring.lock().unwrap());
        this
    }

    /// Builds the entry if there is room for its completion; leaves the
    /// stage `Unbuilt` if not.
    fn try_build(&mut self, ring: &mut Ring) {
        if ring.in_flight >= ring.capacity {
            return;
        }
        let Stage::Unbuilt(op) = std::mem::replace(&mut self.stage, Stage::Finished) else {
            return;
        };
        let file = op.file();
        let (index, generation) = ring.alloc(Kept::new(op));
        let user_data = pack(index, generation);
        let SlotState::Waiting { kept, .. } = &ring.slots[index as usize].state else {
            unreachable!()
        };
        let payload = kept.ptr.cast::<O>();
        // SAFETY: the payload is boxed in the slot and stays there until the
        // completion is popped; nothing else touches it meanwhile.
        let built = unsafe { (*payload).build(&mut BuildCtx { ring, user_data }) };
        match built {
            Ok(()) => {
                ring.in_flight += 1;
                ring.unsubmitted += 1;
                self.stage = Stage::Built {
                    index,
                    generation,
                    file,
                };
            }
            Err(stop) => {
                let SlotState::Waiting { kept, .. } =
                    std::mem::replace(&mut ring.slots[index as usize].state, SlotState::Free)
                else {
                    unreachable!()
                };
                ring.release(index);
                // SAFETY: made from an `O` just above.
                let op = unsafe { kept.take::<O>() };
                self.stage = match stop {
                    // Back to waiting; `poll` registers the waker.
                    BuildStop::Wait => Stage::Unbuilt(op),
                    BuildStop::Fail(e) => Stage::Failed(op, e),
                };
            }
        }
    }
}

impl<O: RingOperation> Future for RingOp<O> {
    type Output = O::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<O::Output> {
        let this = &mut *self;
        let inner = this.inner.clone();
        let mut ring = inner.ring.lock().unwrap();
        if let Stage::Unbuilt(_) = this.stage {
            this.try_build(&mut ring);
            if let Stage::Unbuilt(_) = this.stage {
                ring.room_waiters.push(cx.waker().clone());
                return Poll::Pending;
            }
        }
        match std::mem::replace(&mut this.stage, Stage::Finished) {
            Stage::Failed(op, e) => Poll::Ready(op.fail(e)),
            Stage::Built {
                index,
                generation,
                file,
            } => {
                if let Err(e) = submit_locked(&mut ring) {
                    // Nothing went; the entry stays queued for the next
                    // submission, so keep waiting rather than lose it.
                    let _ = e;
                }
                let slot = &mut ring.slots[index as usize];
                match &mut slot.state {
                    SlotState::Done { .. } => {
                        let SlotState::Done {
                            result,
                            information,
                            kept,
                        } = std::mem::replace(&mut slot.state, SlotState::Free)
                        else {
                            unreachable!()
                        };
                        ring.release(index);
                        drop(ring);
                        // SAFETY: the slot was filled from an `O`.
                        let op = unsafe { kept.take::<O>() };
                        Poll::Ready(op.finish(result, information))
                    }
                    SlotState::Waiting { waker, .. } => {
                        *waker = Some(cx.waker().clone());
                        this.stage = Stage::Built {
                            index,
                            generation,
                            file,
                        };
                        Poll::Pending
                    }
                    SlotState::Free => unreachable!("a built operation's slot was freed"),
                }
            }
            Stage::Unbuilt(_) | Stage::Finished => panic!("RingOp polled after completion"),
        }
    }
}

impl<O: RingOperation> Drop for RingOp<O> {
    fn drop(&mut self) {
        let Stage::Built {
            index,
            generation,
            file,
        } = std::mem::replace(&mut self.stage, Stage::Finished)
        else {
            return;
        };
        let mut ring = self.inner.ring.lock().unwrap();
        if let SlotState::Done { .. } = ring.slots[index as usize].state {
            let state = std::mem::replace(&mut ring.slots[index as usize].state, SlotState::Free);
            ring.release(index);
            drop(ring);
            drop(state);
            return;
        }
        if !self.wait_on_drop {
            // The buffer stays in the slot until the kernel is done; the
            // drain drops it then.
            if let SlotState::Waiting { abandoned, .. } = &mut ring.slots[index as usize].state {
                *abandoned = true;
            }
            let _ = submit_locked(&mut ring);
            ring.cancel(file, pack(index, generation));
            return;
        }
        // A borrowed buffer: the kernel must be done before this returns.
        // Pop completions here, on this thread, until ours has arrived.
        let _ = submit_locked(&mut ring);
        ring.cancel(file, pack(index, generation));
        loop {
            let (_, wake, dead) = drain_locked(&mut ring);
            let done = matches!(ring.slots[index as usize].state, SlotState::Done { .. });
            if done {
                let state = std::mem::replace(&mut ring.slots[index as usize].state, SlotState::Free);
                ring.release(index);
                drop(ring);
                wake.into_iter().for_each(Waker::wake);
                drop(dead);
                drop(state);
                return;
            }
            drop(ring);
            wake.into_iter().for_each(Waker::wake);
            drop(dead);
            std::thread::yield_now();
            ring = self.inner.ring.lock().unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Registered files and buffers
// ---------------------------------------------------------------------------

/// Files registered with a ring: see [`IoRing::register_files`].
///
/// A registered file is named to the kernel by index instead of by handle,
/// which saves looking the handle up on every operation. The ring keeps the
/// handles open itself -- measured, a registration does not: closing the
/// caller's handle after registering made the index fail with
/// `ERROR_INVALID_HANDLE`.
#[derive(Clone)]
pub struct RegisteredFiles {
    table: Arc<FileTable>,
}

struct FileTable {
    ring: usize,
    generation: u64,
    handles: Vec<HANDLE>,
    /// Whatever keeps each handle open.
    _keep: Vec<Box<dyn Send + Sync>>,
}

// Handle values and the owners that keep them open; never mutated.
unsafe impl Send for FileTable {}
unsafe impl Sync for FileTable {}

impl RegisteredFiles {
    /// The `index`th registered file, as a handle operations take.
    pub fn get(&self, index: usize) -> Option<RegisteredFile> {
        (index < self.table.handles.len()).then(|| RegisteredFile {
            table: self.table.clone(),
            index: index as u32,
        })
    }

    pub fn len(&self) -> usize {
        self.table.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.handles.is_empty()
    }
}

/// One registered file; any operation that takes a [`RingHandle`] takes
/// this, and names the file by index. Valid on its own ring while its
/// registration is the active one: once [`IoRing::register_files`] replaces
/// it, operations on this fail with `E_INVALIDARG` rather than reach
/// whatever the new table has at the same index.
#[derive(Clone)]
pub struct RegisteredFile {
    table: Arc<FileTable>,
    index: u32,
}

unsafe impl RingHandle for RegisteredFile {
    fn raw_handle(&self) -> HANDLE {
        self.table.handles[self.index as usize]
    }

    fn ring_registration(&self) -> Option<FileRegistration> {
        Some(FileRegistration {
            ring: self.table.ring,
            generation: self.table.generation,
            index: self.index,
        })
    }
}

/// Registers `files` as one table: see [`IoRing::register_files`].
pub struct RegisterFilesOp {
    table: Arc<FileTable>,
}

impl RingOperation for RegisterFilesOp {
    type Output = Result<RegisteredFiles>;

    fn file(&self) -> IORING_HANDLE_REF {
        IORING_HANDLE_REF::default()
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        // One replacement at a time: a second would race the first's
        // confirmation.
        if ctx.ring.files.pending.is_some() {
            return Err(BuildStop::Wait);
        }
        let table = self.table.clone();
        ctx.entry(|h, user_data| unsafe { BuildIoRingRegisterFileHandles(h, &table.handles, user_data) })?;
        ctx.ring.files.pending = Some(PendingRegistration {
            user_data: ctx.user_data,
            generation: self.table.generation,
            table: self.table.clone(),
        });
        Ok(())
    }

    fn finish(self, result: HRESULT, _information: usize) -> Self::Output {
        result.ok().map(|()| RegisteredFiles { table: self.table })
    }

    fn fail(self, error: Error) -> Self::Output {
        Err(error)
    }
}

/// Buffers registered with a ring: see [`IoRing::register_buffers`].
///
/// Registered memory is named to the kernel by index and offset instead of
/// by address. Each buffer is handed out whole, one holder at a time, as a
/// [`RingBuffer`] -- which is how the registered memory stays out of two
/// writers' hands -- and comes back when that is dropped.
#[derive(Clone)]
pub struct RegisteredBuffers {
    table: Arc<BufferTable>,
}

struct BufferTable {
    ring: usize,
    generation: u64,
    infos: Vec<IORING_BUFFER_INFO>,
    /// The owners of the memory; only ever dropped.
    _keep: Vec<Box<dyn Send>>,
    free: Mutex<Vec<u32>>,
}

// The memory is reached only through `RingBuffer`s, one per buffer at a
// time; the owners are never touched again until dropped.
unsafe impl Send for BufferTable {}
unsafe impl Sync for BufferTable {}

impl RegisteredBuffers {
    /// Any free buffer, or `None` when every one is out.
    pub fn take(&self) -> Option<RingBuffer> {
        let index = self.table.free.lock().unwrap().pop()?;
        Some(RingBuffer {
            table: self.table.clone(),
            index,
            len: 0,
        })
    }

    /// The `index`th buffer, if it is free.
    pub fn take_index(&self, index: usize) -> Option<RingBuffer> {
        let mut free = self.table.free.lock().unwrap();
        let at = free.iter().position(|&i| i as usize == index)?;
        free.swap_remove(at);
        Some(RingBuffer {
            table: self.table.clone(),
            index: index as u32,
            len: 0,
        })
    }

    /// How many buffers were registered.
    pub fn len(&self) -> usize {
        self.table.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.infos.is_empty()
    }

    /// How many are free right now.
    pub fn available(&self) -> usize {
        self.table.free.lock().unwrap().len()
    }
}

/// One registered buffer, held exclusively until dropped. An [`IoBuf`] and
/// [`IoBufMut`] like any other -- reads and writes on its ring address it by
/// index, everything else by its address -- and so is a
/// [`Slice`](crate::common::buffer::Slice) of it.
///
/// Starts empty: `len` is what holds data, and a read sets it.
pub struct RingBuffer {
    table: Arc<BufferTable>,
    index: u32,
    len: usize,
}

impl RingBuffer {
    /// Sets how much of the buffer holds data, up to its size: before a
    /// write of what was filled in by hand.
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(self.capacity());
    }

    /// The buffer's registered size.
    pub fn capacity(&self) -> usize {
        self.table.infos[self.index as usize].Length as usize
    }

    fn base(&self) -> *mut u8 {
        self.table.infos[self.index as usize].Address.cast()
    }
}

impl core::ops::Deref for RingBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.base(), self.len) }
    }
}

impl core::ops::DerefMut for RingBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.base(), self.len) }
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        self.table.free.lock().unwrap().push(self.index);
    }
}

unsafe impl IoBuf for RingBuffer {
    fn stable_ptr(&self) -> *const u8 {
        self.base()
    }
    fn len(&self) -> usize {
        self.len
    }
    fn bytes_total(&self) -> usize {
        self.capacity()
    }
    fn ring_registration(&self) -> Option<crate::common::buffer::RingRegistration> {
        Some(crate::common::buffer::RingRegistration {
            ring: self.table.ring,
            generation: self.table.generation,
            index: self.index,
            offset: 0,
        })
    }
}

unsafe impl IoBufMut for RingBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.base()
    }
    unsafe fn set_init(&mut self, n: usize) {
        self.len = self.len.max(n);
    }
}

/// Registers buffers as one table: see [`IoRing::register_buffers`].
pub struct RegisterBuffersOp {
    table: Arc<BufferTable>,
}

impl RingOperation for RegisterBuffersOp {
    type Output = Result<RegisteredBuffers>;

    fn file(&self) -> IORING_HANDLE_REF {
        IORING_HANDLE_REF::default()
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        if ctx.ring.buffers.pending.is_some() {
            return Err(BuildStop::Wait);
        }
        let table = self.table.clone();
        ctx.entry(|h, user_data| unsafe { BuildIoRingRegisterBuffers(h, &table.infos, user_data) })?;
        ctx.ring.buffers.pending = Some(PendingRegistration {
            user_data: ctx.user_data,
            generation: self.table.generation,
            table: self.table.clone(),
        });
        Ok(())
    }

    fn finish(self, result: HRESULT, _information: usize) -> Self::Output {
        result.ok().map(|()| RegisteredBuffers { table: self.table })
    }

    fn fail(self, error: Error) -> Self::Output {
        Err(error)
    }
}

impl IoRing {
    /// Registers `files`, replacing any files registered before; resolves to
    /// the table, from which [`RegisteredFile`]s are taken. If the kernel
    /// refuses it, no files are registered afterwards -- the previous table
    /// is gone too.
    ///
    /// The ring keeps a clone of each handle for as long as the table is
    /// registered or in use. Operations on files of the replaced table fail
    /// with `E_INVALIDARG` from then on; operations that need registered
    /// files wait while a registration is being confirmed. The kernel has no
    /// way to register nothing (an empty table is refused), so there is no
    /// unregister: replace, or drop the ring.
    pub fn register_files<H: RingHandle + Clone + 'static>(&self, files: &[H]) -> RingOp<RegisterFilesOp> {
        let mut ring = self.inner.ring.lock().unwrap();
        let generation = ring.next_generation;
        ring.next_generation += 1;
        let table = Arc::new(FileTable {
            ring: ring.id,
            generation,
            handles: files.iter().map(RingHandle::raw_handle).collect(),
            _keep: files
                .iter()
                .map(|f| Box::new(f.clone()) as Box<dyn Send + Sync>)
                .collect(),
        });
        drop(ring);
        RingOp::new(&self.inner, RegisterFilesOp { table }, false)
    }

    /// Registers `buffers` -- any owned buffers, each by its whole
    /// [`IoBuf::bytes_total`] -- replacing any registered before; resolves to
    /// the table, from which [`RingBuffer`]s are taken.
    ///
    /// The memory stays registered, and owned by the table, until the table
    /// is replaced and every `RingBuffer` and operation using it is gone.
    /// Fails with `E_INVALIDARG` for a buffer over 4 GiB, which a
    /// registration cannot describe.
    pub fn register_buffers<B: IoBufMut + 'static>(&self, buffers: Vec<B>) -> RingOp<RegisterBuffersOp> {
        let mut infos = Vec::with_capacity(buffers.len());
        let mut keep: Vec<Box<dyn Send>> = Vec::with_capacity(buffers.len());
        let mut too_big = false;
        for mut b in buffers {
            let Ok(len) = u32::try_from(b.bytes_total()) else {
                too_big = true;
                break;
            };
            infos.push(IORING_BUFFER_INFO {
                Address: b.stable_mut_ptr().cast(),
                Length: len,
            });
            keep.push(Box::new(b));
        }
        self.register_infos(infos, keep, too_big)
    }

    /// Registers one region of `count` buffers of `size` bytes each -- a slab
    /// carved from a single page-aligned allocation.
    pub fn register_slab(&self, size: u32, count: u32) -> Result<RingOp<RegisterBuffersOp>> {
        let total = (size as usize)
            .checked_mul(count as usize)
            .ok_or_else(|| Error::new(windows::Win32::Foundation::E_INVALIDARG, "slab too large"))?;
        let region = crate::common::buffer::Region::new(total.max(1))?;
        let infos = (0..count)
            .map(|i| IORING_BUFFER_INFO {
                Address: unsafe { region.base().add(i as usize * size as usize) }.cast(),
                Length: size,
            })
            .collect();
        Ok(self.register_infos(infos, vec![Box::new(region)], false))
    }

    fn register_infos(
        &self,
        infos: Vec<IORING_BUFFER_INFO>,
        keep: Vec<Box<dyn Send>>,
        too_big: bool,
    ) -> RingOp<RegisterBuffersOp> {
        let mut ring = self.inner.ring.lock().unwrap();
        let generation = ring.next_generation;
        ring.next_generation += 1;
        let count = infos.len() as u32;
        let table = Arc::new(BufferTable {
            ring: ring.id,
            generation,
            infos,
            _keep: keep,
            free: Mutex::new((0..count).rev().collect()),
        });
        drop(ring);
        if too_big {
            return RingOp::failed(
                &self.inner,
                RegisterBuffersOp { table },
                Error::new(windows::Win32::Foundation::E_INVALIDARG, "a buffer over 4 GiB"),
            );
        }
        RingOp::new(&self.inner, RegisterBuffersOp { table }, false)
    }
}

// ---------------------------------------------------------------------------
// The pump
// ---------------------------------------------------------------------------

/// The thread-pool wait that drains the ring when its event fires.
struct Pump {
    wait: PTP_WAIT,
    /// `Some` exactly when the environment has no `FinalizationCallback` to
    /// release the context ([`PoolRef::finalizes`]).
    ctx_to_release: Option<*const PumpCtx>,
}

// A thread-pool object and a pointer this pump owns.
unsafe impl Send for Pump {}

/// The pump callback's context. Starts with a [`CtxHeader`] because a `tp`
/// environment's finalization callback treats every context as one.
#[repr(C)]
struct PumpCtx {
    header: CtxHeader,
    /// Weak: the ring's owners decide when it goes, not its pump.
    inner: Weak<Inner>,
    event: HANDLE,
    /// The pool the wait lives in; released with the context, after the wait
    /// is closed.
    _pool: PoolRef,
}

unsafe impl Send for PumpCtx {}
unsafe impl Sync for PumpCtx {}

unsafe fn release_ctx(ctx: *mut core::ffi::c_void) {
    unsafe { drop(Arc::from_raw(ctx as *const PumpCtx)) }
}

thread_local! {
    /// Set while this thread runs a pump callback: a teardown reached from
    /// inside one must not wait for the callback it is running in.
    static IN_PUMP: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

impl Pump {
    fn start(inner: &Arc<Inner>, pool: PoolRef) -> Result<Pump> {
        // Read before the pool moves into the context.
        let env = pool.env();
        let release_ourselves = !pool.finalizes();
        let ctx = Arc::into_raw(Arc::new(PumpCtx {
            header: unsafe { CtxHeader::new(release_ctx) },
            inner: Arc::downgrade(inner),
            event: inner.event,
            _pool: pool,
        }));
        let wait = match unsafe {
            CreateThreadpoolWait(Some(pump_callback), Some(ctx as *mut core::ffi::c_void), env)
        } {
            Ok(wait) => wait,
            Err(e) => {
                unsafe { drop(Arc::from_raw(ctx)) };
                return Err(e);
            }
        };
        unsafe { SetThreadpoolWait(wait, Some(inner.event), None) };
        Ok(Pump {
            wait,
            ctx_to_release: release_ourselves.then_some(ctx),
        })
    }

    /// Stops the wait and frees the context. Must not run inside the pump's
    /// own callback.
    fn stop(self) {
        unsafe {
            SetThreadpoolWait(self.wait, None, None);
            WaitForThreadpoolWaitCallbacks(self.wait, true);
            CloseThreadpoolWait(self.wait);
        }
        if let Some(ctx) = self.ctx_to_release {
            unsafe { drop(Arc::from_raw(ctx)) };
        }
    }
}

struct InPump;

impl Drop for InPump {
    fn drop(&mut self) {
        IN_PUMP.with(|f| f.set(false));
    }
}

unsafe extern "system" fn pump_callback(
    instance: PTP_CALLBACK_INSTANCE,
    ctx: *mut core::ffi::c_void,
    wait: PTP_WAIT,
    _result: u32,
) {
    // A drain can run as long as completions keep coming.
    let _ = unsafe { CallbackMayRunLong(instance) };
    IN_PUMP.with(|f| f.set(true));
    let _in_pump = InPump;
    let ctx = unsafe { &*(ctx as *const PumpCtx) };
    let Some(inner) = ctx.inner.upgrade() else {
        return;
    };
    let (wake, dead) = {
        let mut ring = inner.ring.lock().unwrap();
        let (_, wake, dead) = drain_locked(&mut ring);
        // Re-armed while the ring is certainly alive. A completion posted
        // after the last pop has signalled the (auto-reset) event, so the
        // wait fires again at once rather than missing it.
        unsafe { SetThreadpoolWait(wait, Some(ctx.event), None) };
        (wake, dead)
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wake.into_iter().for_each(Waker::wake);
        drop(dead);
    }));
    // If this was the last owner, `Inner::drop` runs here, with `IN_PUMP`
    // set, and leaves stopping this wait to another thread.
    drop(inner);
}

// ---------------------------------------------------------------------------
// Teardown
// ---------------------------------------------------------------------------

impl Drop for Inner {
    fn drop(&mut self) {
        let pump = self.pump.get_mut().unwrap().take();
        let ring = self.ring.get_mut().unwrap();

        // Every operation still in the ring belongs to a future that is gone,
        // and each was cancelled when its future dropped. Wait for the kernel
        // to finish with their buffers; give up after a while rather than
        // hang a destructor, and leak what is left rather than free memory
        // the kernel may still write.
        let _ = submit_locked(ring);
        let mut rounds = 0;
        while ring.in_flight > 0 && rounds < TEARDOWN_ROUNDS {
            let (n, wake, dead) = drain_locked(ring);
            wake.into_iter().for_each(Waker::wake);
            drop(dead);
            if n == 0 {
                rounds += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        if ring.in_flight > 0 {
            for slot in ring.slots.drain(..) {
                if let SlotState::Waiting { kept, .. } = slot.state {
                    std::mem::forget(kept);
                }
            }
        }

        let handle = ring.handle;
        let event = self.event;
        let close = move || unsafe {
            let _ = CloseIoRing(handle);
            let _ = CloseHandle(event);
        };
        match pump {
            None => close(),
            Some(pump) if IN_PUMP.with(|f| f.get()) => {
                // Inside our own callback: waiting for it would be waiting
                // for ourselves. Another thread stops it moments later.
                struct Later(Pump, HIORING, HANDLE);
                unsafe impl Send for Later {}
                impl Later {
                    fn run(self) {
                        let Later(pump, handle, event) = self;
                        pump.stop();
                        unsafe {
                            let _ = CloseIoRing(handle);
                            let _ = CloseHandle(event);
                        }
                    }
                }
                let later = Later(pump, handle, event);
                // A method call, so the closure captures `later` whole rather
                // than its non-`Send` fields one by one.
                std::thread::spawn(move || later.run());
            }
            Some(pump) => {
                pump.stop();
                close();
            }
        }
    }
}

#[cfg(test)]
mod tests;
