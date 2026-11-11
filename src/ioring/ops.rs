//! The operations a ring builds -- read, write, flush -- and the future
//! that waits for one.

use super::*;

/// What a ring operation knows how to do: build its entry, and turn the
/// completion back into a result.
pub trait RingOperation: Sized {
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
    pub(super) file: H,
    pub(super) buf: B,
    pub(super) offset: u64,
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
    pub(super) file: H,
    pub(super) buf: B,
    pub(super) offset: u64,
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
    pub(super) file: H,
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

/// One operation on an [`IoRing`], as a future. `!Send`, like its ring.
///
/// Built when created (so operations created together are submitted
/// together), submitted when first polled. Each poll also pops the ring's
/// completion queue, so a task awaiting an operation is the ring's driver
/// for as long as it waits. Dropping it before it completes cancels the
/// operation; see [`IoRing`] for what happens to the buffer.
pub struct RingOp<O: RingOperation> {
    pub(super) inner: Rc<Inner>,
    pub(super) stage: Stage<O>,
    /// Set for the `*_unchecked` operations: a drop waits for the kernel.
    pub(super) wait_on_drop: bool,
}

pub(super) enum Stage<O> {
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

impl<O: RingOperation> Unpin for RingOp<O> {}

impl<O: RingOperation> RingOp<O> {
    /// An operation that resolves to `error` without being built.
    pub(super) fn failed(inner: &Rc<Inner>, op: O, error: Error) -> Self {
        RingOp {
            inner: inner.clone(),
            stage: Stage::Failed(op, error),
            wait_on_drop: false,
        }
    }

    pub(super) fn new(inner: &Rc<Inner>, op: O, wait_on_drop: bool) -> Self {
        let mut this = RingOp {
            inner: inner.clone(),
            stage: Stage::Unbuilt(op),
            wait_on_drop,
        };
        this.try_build(&mut inner.ring.borrow_mut());
        this
    }

    /// [`Future::poll`], with the pop of the completion queue optional: a
    /// [`Batch`] pops once and then polls its operations without.
    pub(super) fn poll_inner(&mut self, cx: &mut Context<'_>, pop: bool) -> Poll<O::Output> {
        let inner = self.inner.clone();
        // Whoever polls last is the one the pump wakes.
        inner.driver.register(cx.waker());
        let mut ring = inner.ring.borrow_mut();
        if let Stage::Unbuilt(_) = self.stage {
            self.try_build(&mut ring);
            if let Stage::Unbuilt(_) = self.stage {
                ring.room_waiters.push(cx.waker().clone());
                return Poll::Pending;
            }
        }
        match std::mem::replace(&mut self.stage, Stage::Finished) {
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
                let (mut wake, dead) = if pop {
                    let (_, wake, dead) = drain_locked(&mut ring);
                    (wake, dead)
                } else {
                    (Vec::new(), Vec::new())
                };
                let slot = &mut ring.slots[index as usize];
                let result = match &mut slot.state {
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
                        // This task may stop polling now: hand the wake on.
                        wake.append(&mut ring.other_waiters(Some(cx.waker())));
                        drop(ring);
                        // SAFETY: the slot was filled from an `O`.
                        let op = unsafe { kept.take::<O>() };
                        Poll::Ready(op.finish(result, information))
                    }
                    SlotState::Waiting { waker, .. } => {
                        *waker = Some(cx.waker().clone());
                        self.stage = Stage::Built {
                            index,
                            generation,
                            file,
                        };
                        drop(ring);
                        Poll::Pending
                    }
                    SlotState::Free => unreachable!("a built operation's slot was freed"),
                };
                // The drain's wakers are not ours to skip: a task woken here
                // re-polls and finds its result. Both run with the borrow
                // released, since either may poll into this ring.
                wake.into_iter().filter(|w| !w.will_wake(cx.waker())).for_each(Waker::wake);
                drop(dead);
                result
            }
            Stage::Unbuilt(_) | Stage::Finished => panic!("RingOp polled after completion"),
        }
    }

    /// Builds the entry if there is room for its completion; leaves the
    /// stage `Unbuilt` if not.
    pub(super) fn try_build(&mut self, ring: &mut Ring) {
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
                ring.publish_in_flight();
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
        self.poll_inner(cx, true)
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
        let mut ring = self.inner.ring.borrow_mut();
        // The task that owned this future may stop polling the ring now, and
        // it may have been the one the pump would wake: pass the baton.
        let mut wake = ring.other_waiters(None);
        if let SlotState::Done { .. } = ring.slots[index as usize].state {
            let state = std::mem::replace(&mut ring.slots[index as usize].state, SlotState::Free);
            ring.release(index);
            drop(ring);
            wake.into_iter().for_each(Waker::wake);
            drop(state);
            return;
        }
        if !self.wait_on_drop {
            // The buffer stays in the slot until the kernel is done; the
            // drain drops it then.
            if let SlotState::Waiting { abandoned, waker, .. } = &mut ring.slots[index as usize].state {
                *abandoned = true;
                *waker = None;
            }
            let _ = submit_locked(&mut ring);
            ring.cancel(file, pack(index, generation));
            drop(ring);
            wake.into_iter().for_each(Waker::wake);
            return;
        }
        // A borrowed buffer: the kernel must be done before this returns.
        // Pop completions here, on this thread, until ours has arrived.
        let _ = submit_locked(&mut ring);
        ring.cancel(file, pack(index, generation));
        loop {
            let (_, mut woken, dead) = drain_locked(&mut ring);
            wake.append(&mut woken);
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
            std::mem::take(&mut wake).into_iter().for_each(Waker::wake);
            drop(dead);
            std::thread::yield_now();
            ring = self.inner.ring.borrow_mut();
        }
    }
}

// ---------------------------------------------------------------------------
// Registered files and buffers
// ---------------------------------------------------------------------------
