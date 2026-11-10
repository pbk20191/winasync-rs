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
//! # Capacity
//!
//! The completion queue is twice the submission queue, and no more
//! operations are ever in flight than it can hold -- an overflowing
//! completion queue would lose results. An operation created at that limit
//! waits for room before it is built.

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
    BuildIoRingCancelRequest, BuildIoRingFlushFile, BuildIoRingReadFile, BuildIoRingWriteFile,
    CloseIoRing, CreateFileW, CreateIoRing, FILE_ATTRIBUTE_NORMAL, FILE_FLUSH_DEFAULT,
    FILE_SHARE_READ, FILE_WRITE_FLAGS_NONE, HIORING, IORING_BUFFER_REF, IORING_BUFFER_REF_0,
    IORING_CQE, IORING_CREATE_FLAGS, IORING_HANDLE_REF, IORING_HANDLE_REF_0, IORING_REF_RAW,
    IORING_VERSION_3, IOSQE_FLAGS_NONE, CREATE_ALWAYS, OPEN_EXISTING, PopIoRingCompletion,
    SetIoRingCompletionEvent, SubmitIoRing,
};
use windows::Win32::System::Threading::{
    CallbackMayRunLong, CloseThreadpoolWait, CreateEventW, CreateThreadpoolWait,
    SetThreadpoolWait, WaitForThreadpoolWaitCallbacks, PTP_CALLBACK_INSTANCE, PTP_WAIT,
    TP_CALLBACK_ENVIRON_V3,
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

fn handle_ref<H: RingHandle>(file: &H) -> IORING_HANDLE_REF {
    IORING_HANDLE_REF {
        Kind: IORING_REF_RAW,
        Handle: IORING_HANDLE_REF_0 {
            Handle: file.raw_handle(),
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
    /// Operations waiting for `in_flight` to drop below `capacity`.
    room_waiters: Vec<Waker>,
}

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
        Self::build(entries, Some(None), None)
    }

    /// [`new`](Self::new), with the pump on `rt`'s pool.
    #[cfg(feature = "tp")]
    pub fn with_runtime(rt: &crate::tp::Runtime, entries: u32) -> Result<IoRing> {
        Self::build(
            entries,
            Some(Some(rt.inner.pool.default_env())),
            Some(rt.inner.clone()),
        )
    }

    /// A ring with no pump: completions are delivered only when the caller
    /// calls [`drive`](Self::drive).
    pub fn manual(entries: u32) -> Result<IoRing> {
        Self::build(entries, None, None)
    }

    /// `pump` is `None` for a manual ring, else the callback environment
    /// (`None` inside for the process default pool).
    fn build(
        entries: u32,
        pump: Option<Option<*const TP_CALLBACK_ENVIRON_V3>>,
        keepalive: Option<Arc<dyn core::any::Any + Send + Sync>>,
    ) -> Result<IoRing> {
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
            }),
            event,
            pump: Mutex::new(None),
        });
        if let Some(env) = pump {
            let pump = Pump::start(&inner, env, keepalive)?;
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
    fn build(&mut self, ring: HIORING, user_data: usize) -> Result<()>;

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

    fn build(&mut self, ring: HIORING, user_data: usize) -> Result<()> {
        let len = self.buf.bytes_total() as u32;
        let data = IORING_BUFFER_REF {
            Kind: IORING_REF_RAW,
            Buffer: IORING_BUFFER_REF_0 {
                Address: self.buf.stable_mut_ptr().cast(),
            },
        };
        unsafe {
            BuildIoRingReadFile(ring, handle_ref(&self.file), data, len, self.offset, user_data, IOSQE_FLAGS_NONE)
        }
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

    fn build(&mut self, ring: HIORING, user_data: usize) -> Result<()> {
        let len = IoBuf::len(&self.buf) as u32;
        let data = IORING_BUFFER_REF {
            Kind: IORING_REF_RAW,
            Buffer: IORING_BUFFER_REF_0 {
                // One pointer type for both directions; a write only reads it.
                Address: self.buf.stable_ptr() as *mut core::ffi::c_void,
            },
        };
        unsafe {
            BuildIoRingWriteFile(
                ring,
                handle_ref(&self.file),
                data,
                len,
                self.offset,
                FILE_WRITE_FLAGS_NONE,
                user_data,
                IOSQE_FLAGS_NONE,
            )
        }
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

    fn build(&mut self, ring: HIORING, user_data: usize) -> Result<()> {
        unsafe { BuildIoRingFlushFile(ring, handle_ref(&self.file), FILE_FLUSH_DEFAULT, user_data, IOSQE_FLAGS_NONE) }
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
    /// Not built yet: the completion queue was full.
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
        let built = build_with_retry(ring, |h| unsafe { (*payload).build(h, user_data) });
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
            Err(e) => {
                let SlotState::Waiting { kept, .. } =
                    std::mem::replace(&mut ring.slots[index as usize].state, SlotState::Free)
                else {
                    unreachable!()
                };
                ring.release(index);
                // SAFETY: made from an `O` just above.
                self.stage = Stage::Failed(unsafe { kept.take::<O>() }, e);
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
// The pump
// ---------------------------------------------------------------------------

/// The thread-pool wait that drains the ring when its event fires.
struct Pump {
    wait: PTP_WAIT,
    /// `Some` exactly when the callback environment is null: only an
    /// environment carries a `FinalizationCallback` to release the context.
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
    _keepalive: Option<Arc<dyn core::any::Any + Send + Sync>>,
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
    fn start(
        inner: &Arc<Inner>,
        env: Option<*const TP_CALLBACK_ENVIRON_V3>,
        keepalive: Option<Arc<dyn core::any::Any + Send + Sync>>,
    ) -> Result<Pump> {
        let ctx = Arc::into_raw(Arc::new(PumpCtx {
            header: unsafe { CtxHeader::new(release_ctx) },
            inner: Arc::downgrade(inner),
            event: inner.event,
            _keepalive: keepalive,
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
            ctx_to_release: env.is_none().then_some(ctx),
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
mod tests {
    use super::*;
    use crate::common::block_on;
    use std::time::{Duration, Instant};

    fn temp_file(tag: &str, contents: &[u8]) -> (String, File) {
        let path = std::env::temp_dir()
            .join(format!("ioring_{tag}_{}.bin", std::process::id()))
            .to_string_lossy()
            .into_owned();
        std::fs::write(&path, contents).unwrap();
        let file = File::open(&path).unwrap();
        (path, file)
    }

    fn in_flight(ring: &IoRing) -> u32 {
        ring.inner.ring.lock().unwrap().in_flight
    }

    /// Waits for the ring to have nothing in flight -- cancelled operations
    /// still have to complete.
    fn settle(ring: &IoRing) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while in_flight(ring) > 0 && Instant::now() < deadline {
            ring.drive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(in_flight(ring), 0, "operations were left in flight");
    }

    #[test]
    fn write_then_read() {
        let ring = IoRing::new(8).unwrap();
        let (path, file) = temp_file("rw", b"");
        let (n, _) = block_on(ring.write(&file, b"ring-written".to_vec(), 0));
        assert_eq!(n.unwrap(), 12);
        // Spare capacity, not zeroed bytes: the length ends at the data.
        let (n, got) = block_on(ring.read(&file, Vec::with_capacity(64), 0));
        assert_eq!(n.unwrap(), 12);
        assert_eq!(got, b"ring-written");
        block_on(ring.flush(&file)).unwrap();
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reading_past_the_end_reads_nothing() {
        let ring = IoRing::new(4).unwrap();
        let (path, file) = temp_file("eof", b"short");
        let (n, _) = block_on(ring.read(&file, vec![0u8; 16], 1000));
        assert_eq!(n.unwrap(), 0);
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// The point of a ring: operations created together go to the kernel in
    /// one submission.
    #[test]
    fn operations_created_together_are_submitted_together() {
        let ring = IoRing::new(32).unwrap();
        let data: Vec<u8> = (0..16u8).flat_map(|i| [i; 512]).collect();
        let (path, file) = temp_file("batch", &data);
        let ops: Vec<_> = (0..16u64)
            .map(|i| ring.read(&file, Vec::with_capacity(512), i * 512))
            .collect();
        assert_eq!(ring.submit().unwrap(), 16, "the reads did not go as one batch");
        for (i, op) in ops.into_iter().enumerate() {
            let (n, got) = block_on(op);
            assert_eq!(n.unwrap(), 512);
            assert!(got.iter().all(|&b| b == i as u8), "read {i} got the wrong bytes");
        }
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// More operations than the completion queue holds: the rest wait for
    /// room instead of overflowing it, and all of them finish.
    #[test]
    fn operations_past_the_queue_wait_for_room() {
        let ring = IoRing::new(1).unwrap();
        let (path, file) = temp_file("room", &[7u8; 64]);
        let ops: Vec<_> = (0..10u64).map(|i| ring.read(&file, vec![0u8; 4], i * 4)).collect();
        assert!(in_flight(&ring) <= 2, "more in flight than the completion queue holds");
        for op in ops {
            let (n, got) = block_on(op);
            assert_eq!(n.unwrap(), 4);
            assert_eq!(&got[..4], &[7u8; 4]);
        }
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// Without a pump nothing arrives until the caller drives the ring.
    #[test]
    fn a_manual_ring_delivers_when_driven() {
        let ring = IoRing::manual(4).unwrap();
        let (path, file) = temp_file("manual", b"driven");
        let mut op = ring.read(&file, Vec::with_capacity(16), 0);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(Pin::new(&mut op).poll(&mut cx).is_pending());
        let deadline = Instant::now() + Duration::from_secs(5);
        let (n, got) = loop {
            ring.drive();
            if let Poll::Ready(r) = Pin::new(&mut op).poll(&mut cx) {
                break r;
            }
            assert!(Instant::now() < deadline, "never completed");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(&got[..n.unwrap()], b"driven");
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// A future dropped before it finishes leaves nothing behind once the
    /// kernel is done with its buffer.
    #[test]
    fn a_dropped_operation_is_settled() {
        let ring = IoRing::new(8).unwrap();
        let (path, file) = temp_file("drop", &[1u8; 4096]);
        for i in 0..8u64 {
            drop(ring.read(&file, vec![0u8; 512], i * 512));
        }
        settle(&ring);
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// Borrowed buffers, and an early drop that returns.
    #[test]
    fn unchecked_operations_take_borrowed_buffers() {
        let ring = IoRing::new(4).unwrap();
        let (path, file) = temp_file("borrow", b"");
        let out = *b"borrowed";
        let mut back = [0u8; 8];
        // SAFETY: each future is awaited or dropped, never forgotten.
        unsafe {
            let (n, _) = block_on(ring.write_unchecked(&file, &out[..], 0));
            assert_eq!(n.unwrap(), 8);
            let (n, _) = block_on(ring.read_unchecked(&file, &mut back[..], 0));
            assert_eq!(n.unwrap(), 8);
            drop(ring.read_unchecked(&file, &mut back[..], 0));
        }
        assert_eq!(&back, b"borrowed");
        settle(&ring);
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// Slices and foreign memory work like any other buffer.
    #[test]
    fn slices_and_raw_buffers() {
        use crate::common::buffer::{RawBuf, Region};
        let ring = IoRing::new(4).unwrap();
        let (path, file) = temp_file("bufs", b"0123456789");
        let mut head = Vec::with_capacity(16);
        head.extend_from_slice(b"at:");
        let (n, rest) = block_on(ring.read(&file, head.slice(3..7), 2));
        assert_eq!(n.unwrap(), 4);
        assert_eq!(rest.into_inner(), b"at:2345");
        let region = Region::new(4096).unwrap();
        let raw = unsafe { RawBuf::from_raw_parts(region.base(), 0, 4096, region) };
        let (n, raw) = block_on(ring.read(&file, raw, 0));
        assert_eq!(n.unwrap(), 10);
        assert_eq!(raw.as_slice(), b"0123456789");
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(feature = "tp")]
    #[test]
    fn pumped_by_a_tp_runtime() {
        let rt = crate::tp::Runtime::new();
        let ring = IoRing::with_runtime(&rt, 4).unwrap();
        let (path, file) = temp_file("tp", b"on tp's pool");
        let (n, got) = block_on(ring.read(&file, Vec::with_capacity(32), 0));
        assert_eq!(&got[..n.unwrap()], b"on tp's pool");
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// The ring goes with its last owner, operations it still holds included.
    #[test]
    fn a_ring_with_abandoned_operations_tears_down() {
        let (path, file) = temp_file("teardown", &[2u8; 8192]);
        let ring = IoRing::new(8).unwrap();
        for i in 0..8u64 {
            drop(ring.read(&file, vec![0u8; 1024], i * 1024));
        }
        let started = Instant::now();
        drop(ring);
        assert!(started.elapsed() < Duration::from_secs(5), "teardown hung");
        drop(file);
        let _ = std::fs::remove_file(&path);
    }

    /// Shared std files, borrowed std files and borrowed handles, overlapped
    /// or not.
    #[test]
    fn handles_of_every_kind() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsHandle;
        let ring = IoRing::new(8).unwrap();
        let (path, file) = temp_file("handles", b"handles!");
        drop(file);

        let shared = Arc::new(std::fs::File::open(&path).unwrap());
        let (n, got) = block_on(ring.read(&shared, Vec::with_capacity(16), 0));
        assert_eq!(&got[..n.unwrap()], b"handles!", "Arc<std::fs::File>");

        let overlapped = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED.0)
            .open(&path)
            .unwrap();
        let mut buf = [0u8; 16];
        // SAFETY: each future is awaited to completion.
        unsafe {
            let (n, _) = block_on(ring.read_unchecked(&&overlapped, &mut buf[..], 0));
            assert_eq!(&buf[..n.unwrap()], b"handles!", "&std::fs::File, overlapped");
            let borrowed = overlapped.as_handle();
            let (n, _) = block_on(ring.read_unchecked(&borrowed, &mut buf[..], 4));
            assert_eq!(&buf[..n.unwrap()], b"les!", "BorrowedHandle");
        }
        drop((shared, overlapped));
        let _ = std::fs::remove_file(&path);
    }

    /// A handle bound to a completion port is refused, not mis-delivered.
    #[test]
    fn a_port_bound_handle_is_refused() {
        use windows::Win32::System::IO::CreateIoCompletionPort;
        let ring = IoRing::new(4).unwrap();
        let (path, file) = temp_file("port", b"bound");
        drop(file);
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path.as_str()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED,
                None,
            )
            .unwrap()
        };
        let port = unsafe { CreateIoCompletionPort(handle, None, 1, 1).unwrap() };
        // SAFETY: a fresh handle this test owns and hands over.
        let bound = unsafe { File::from_raw_handle(handle) };
        let (r, _) = block_on(ring.read(&bound, vec![0u8; 8], 0));
        assert_eq!(r.unwrap_err().code(), windows::Win32::Foundation::E_INVALIDARG);
        drop(bound);
        unsafe {
            let _ = CloseHandle(port);
        }
        let _ = std::fs::remove_file(&path);
    }
}
