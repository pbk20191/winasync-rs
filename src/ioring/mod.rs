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
//! demand. A [`Batch`] is that pattern as one future: the operations
//! collected into it, submitted by its first poll in one `SubmitIoRing`,
//! resolving to all their results in order -- queue depth from one task
//! at one system call per batch, which is where a ring beats a handle
//! bound to a completion port.
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
    CallbackMayRunLong, CreateEventW, CreateThreadpoolWait, SetThreadpoolWait,
    PTP_CALLBACK_INSTANCE, PTP_WAIT,
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
    files: Registration<FileTable>,
    buffers: Registration<BufferTable>,
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
struct Registration<T> {
    /// The active registration's number; 0 for none.
    generation: u64,
    /// Keeps the active table -- its handles, its memory -- alive while the
    /// kernel may use it.
    active: Option<Arc<T>>,
    pending: Option<PendingRegistration<T>>,
}

impl<T> Default for Registration<T> {
    fn default() -> Self {
        Registration {
            generation: 0,
            active: None,
            pending: None,
        }
    }
}

struct PendingRegistration<T> {
    user_data: usize,
    generation: u64,
    table: Arc<T>,
}

impl<T> Registration<T> {
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

#[cfg(test)]
mod tests;

mod batch;
mod file;
mod ops;
mod pump;
mod registered;
pub use batch::*;
pub use file::*;
pub use ops::*;
use pump::*;
pub use registered::*;
