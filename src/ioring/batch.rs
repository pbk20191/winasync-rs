//! Several operations as one: built together, submitted in one
//! `SubmitIoRing`, resolved to their results in order.

use super::*;

/// Operations of one kind on a ring, awaited as one future that resolves
/// to every result in the order the operations were added.
///
/// Made by collecting the ring's operations -- `ring.read(..)` per offset,
/// then `.collect()` -- or by [`IoRing::read_batch`] and
/// [`IoRing::write_batch`]. Each operation is built as it is created, and
/// the batch's first poll submits all of them in one `SubmitIoRing`: a
/// queue depth of `n` from one task costs one system call, not `n`.
/// Operations past the ring's capacity wait for room and go when it comes.
///
/// ```ignore
/// let reads: Batch<_> = offsets.iter().map(|&o| ring.read(&file, vec![0u8; 4096], o)).collect();
/// for (n, buf) in reads.await { .. }
/// ```
///
/// Dropped before it resolves, a batch cancels what is still in flight, as
/// each [`RingOp`] does on its own.
///
/// One completion wakes the batch, and the batch pops the ring once and
/// re-polls every operation still pending; that is a borrow and a waker
/// clone each, so a batch is for tens or hundreds of operations, not
/// hundreds of thousands.
pub struct Batch<O: RingOperation> {
    /// `None` once the operation has resolved.
    ops: Vec<Option<RingOp<O>>>,
    /// The results, in the operations' order, filled in as they resolve.
    results: Vec<Option<O::Output>>,
    remaining: usize,
    /// Whether the first poll, which submits, has happened.
    submitted: bool,
}

impl<O: RingOperation> Batch<O> {
    /// How many operations the batch holds.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether the batch holds no operation.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// How many have not resolved yet.
    pub fn pending(&self) -> usize {
        self.remaining
    }

    /// Polls every unresolved operation once, without popping, and keeps
    /// the results of those that resolved.
    fn pass(&mut self, cx: &mut Context<'_>) {
        for (op, result) in self.ops.iter_mut().zip(self.results.iter_mut()) {
            let Some(pending) = op else { continue };
            if let Poll::Ready(out) = pending.poll_inner(cx, false) {
                *result = Some(out);
                *op = None;
                self.remaining -= 1;
            }
        }
    }
}

impl<O: RingOperation> FromIterator<RingOp<O>> for Batch<O> {
    fn from_iter<I: IntoIterator<Item = RingOp<O>>>(iter: I) -> Self {
        let ops: Vec<Option<RingOp<O>>> = iter.into_iter().map(Some).collect();
        let remaining = ops.len();
        let results = ops.iter().map(|_| None).collect();
        Batch { ops, results, remaining, submitted: false }
    }
}

impl<O: RingOperation> Unpin for Batch<O> {}

impl<O: RingOperation> Future for Batch<O> {
    type Output = Vec<O::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        assert!(
            this.remaining > 0 || this.ops.is_empty(),
            "Batch polled after completion"
        );
        // The first poll submits, by polling each operation; only then is
        // there anything to pop. A cached read can complete inside
        // `SubmitIoRing` itself, so popping right after the submission
        // finds it without a round trip through the pump -- which is the
        // difference between one wake and none at queue depth 1.
        if !this.submitted {
            this.submitted = true;
            this.pass(cx);
        }
        // One pop for the whole batch, then the operations look at their
        // slots without popping again.
        if let Some(any) = this.ops.iter().flatten().next() {
            any.inner.pop_for(cx.waker());
        }
        this.pass(cx);
        if this.remaining > 0 {
            return Poll::Pending;
        }
        // Mark finished (the empty-batch case included) and hand over.
        this.remaining = usize::MAX;
        Poll::Ready(
            std::mem::take(&mut this.results)
                .into_iter()
                .map(|r| r.expect("a resolved operation left no result"))
                .collect(),
        )
    }
}

impl IoRing {
    /// A [`Batch`] of reads on `file`: one per `(buffer, offset)`, resolving
    /// to the results in that order. See [`read`](Self::read) for one.
    pub fn read_batch<H: RingHandle + Clone + 'static, B: IoBufMut + 'static>(
        &self,
        file: &H,
        reads: impl IntoIterator<Item = (B, u64)>,
    ) -> Batch<ReadOp<H, B>> {
        reads.into_iter().map(|(buf, offset)| self.read(file, buf, offset)).collect()
    }

    /// A [`Batch`] of writes on `file`: one per `(buffer, offset)`, resolving
    /// to the results in that order. See [`write`](Self::write) for one.
    pub fn write_batch<H: RingHandle + Clone + 'static, B: IoBuf + 'static>(
        &self,
        file: &H,
        writes: impl IntoIterator<Item = (B, u64)>,
    ) -> Batch<WriteOp<H, B>> {
        writes.into_iter().map(|(buf, offset)| self.write(file, buf, offset)).collect()
    }
}
