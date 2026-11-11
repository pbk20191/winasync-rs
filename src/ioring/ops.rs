//! The operations a ring builds -- read, write, flush -- and the future
//! that waits for one.

use super::*;

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

/// One operation on an [`IoRing`], as a future.
///
/// Built when created (so operations created together are submitted
/// together), submitted when first polled. Dropping it before it completes
/// cancels the operation; see [`IoRing`] for what happens to the buffer.
pub struct RingOp<O: RingOperation> {
    pub(super) inner: Arc<Inner>,
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

// The handle reference is a plain handle value; the payload is `Send`.
unsafe impl<O: RingOperation> Send for RingOp<O> {}
impl<O: RingOperation> Unpin for RingOp<O> {}

impl<O: RingOperation> RingOp<O> {
    /// An operation that resolves to `error` without being built.
    pub(super) fn failed(inner: &Arc<Inner>, op: O, error: Error) -> Self {
        RingOp {
            inner: inner.clone(),
            stage: Stage::Failed(op, error),
            wait_on_drop: false,
        }
    }

    pub(super) fn new(inner: &Arc<Inner>, op: O, wait_on_drop: bool) -> Self {
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
