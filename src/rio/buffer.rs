//! Registered buffer slabs.
//!
//! RIO's main saving is that the kernel validates and pins a buffer **once**,
//! at `RIORegisterBuffer`, instead of per operation. An op then names a
//! `RIO_BUF { BufferId, Offset, Length }` — a descriptor, not a pointer the
//! kernel must probe again. Everything else about RIO's buffer story follows
//! from that:
//!
//! * A buffer must stay registered and allocated until every operation
//!   referencing it has completed. RIO has **no cancellation** — only
//!   `closesocket`, and even then completions arrive *afterwards* — so
//!   "until the op finishes" genuinely means after the socket is gone.
//!   [`RioSlice`] therefore holds an `Arc` of its pool, which keeps the
//!   registration alive no matter what order things are dropped in.
//!
//! * Registering many small buffers wastes memory: the pinning granularity is
//!   a page, so each registration can cost a whole page. One slab cut into
//!   slices is the shape every real implementation uses.
//!
//! A documentation caveat worth knowing: `RIOSend`'s page says that while a
//! send references a registered buffer, *"the entire registered buffer must
//! not be used"*, including for another operation. Read literally that
//! forbids slicing one registration at all — yet slicing is what Microsoft's
//! own samples and every shipped implementation do, and no clarification
//! exists. This module slices, like everyone else; the alternative would be
//! one registration per concurrent operation, which the page-granularity note
//! above argues against.

use std::sync::Arc;
use windows::Win32::Networking::WinSock::{RIO_BUF, RIO_BUFFERID};

use super::cq::Request;
use super::rq::RIO_INVALID_BUFFERID;
use super::table::rio_table;
use crate::common::buffer::{IoBuf, IoBufMut, SlabBuf, SlabPool};

/// A [`SlabPool`] whose whole region is registered with RIO.
///
/// The slab arithmetic — one reservation, fixed slices, return on drop — is
/// `common`'s; this adds the registration and the per-slice `RIO_BUF`
/// descriptor, which is all RIO actually needs on top.
pub struct RioBufferPool {
    slab: Arc<SlabPool>,
    id: RIO_BUFFERID,
    /// One operation's state per slice, made with the pool. A slice is in
    /// at most one operation at a time, so slot `i` is the state of
    /// whatever operation slice `i` is in — and an operation allocates
    /// nothing.
    slots: Box<[Request]>,
}

impl RioBufferPool {
    /// Allocates and registers a slab of `slice_count` slices of
    /// `slice_size` bytes.
    pub fn new(slice_size: usize, slice_count: u32) -> windows_core::Result<Arc<Self>> {
        let table = rio_table()?;
        let slab = SlabPool::new(slice_size, slice_count)?;
        let (base, len) = slab.region();
        let id = unsafe {
            (table.raw().RIORegisterBuffer.unwrap())(windows_core::PCSTR(base), len as u32)
        };
        if id == RIO_INVALID_BUFFERID {
            return Err(crate::common::winsock::wsa_last_err());
        }
        let slots = (0..slice_count).map(|_| Request::empty()).collect();
        Ok(Arc::new(Self { slab, id, slots }))
    }

    /// The operation slot of `slice` — `slice` must be of this pool.
    pub(crate) fn slot(&self, slice: &RioSlice) -> &Request {
        debug_assert!(core::ptr::eq(&*slice.pool, self));
        &self.slots[slice.buf.index() as usize]
    }

    /// Takes a slice, or `None` when every slice is out.
    pub fn acquire(self: &Arc<Self>) -> Option<RioSlice> {
        let buf = self.slab.acquire()?;
        Some(RioSlice {
            pool: self.clone(),
            buf,
        })
    }

    pub fn available(&self) -> usize {
        self.slab.available()
    }

    pub fn slice_size(&self) -> usize {
        self.slab.slice_size()
    }
}

impl Drop for RioBufferPool {
    /// Deregisters before the slab is freed. Reaching here means no
    /// [`RioSlice`] is alive — each holds an `Arc` of this pool — and
    /// therefore that no operation still references the registration.
    fn drop(&mut self) {
        if let Ok(table) = rio_table() {
            unsafe { (table.raw().RIODeregisterBuffer.unwrap())(self.id) };
        }
        // The slab's own Drop releases the region, after this.
    }
}

/// One slice of a [`RioBufferPool`], returned to it when dropped.
pub struct RioSlice {
    pool: Arc<RioBufferPool>,
    buf: SlabBuf,
}

impl RioSlice {
    /// The descriptor an operation passes to RIO: which registration, where
    /// in it, and how much.
    pub(crate) fn rio_buf(&self) -> RIO_BUF {
        RIO_BUF {
            BufferId: self.pool.id,
            Offset: self.buf.offset() as u32,
            Length: IoBuf::len(&self.buf) as u32,
        }
    }

    /// Narrows the slice to its first `len` bytes — how a send says how much
    /// is payload. Capped at the slice size.
    pub fn set_len(&mut self, len: usize) {
        self.buf.set_len(len);
    }

    /// Restores the slice to its full size, for reuse as a receive buffer.
    pub fn reset_len(&mut self) {
        self.buf.reset_len();
    }

    /// The pool this slice is of.
    pub(crate) fn pool(&self) -> &Arc<RioBufferPool> {
        &self.pool
    }
}

impl core::ops::Deref for RioSlice {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl core::ops::DerefMut for RioSlice {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

// A RIO slice is a perfectly ordinary owned IO buffer; it is only RIO's
// *operations* that insist on this type, because the memory has to be
// registered.
unsafe impl IoBuf for RioSlice {
    fn stable_ptr(&self) -> *const u8 {
        self.buf.stable_ptr()
    }
    fn len(&self) -> usize {
        IoBuf::len(&self.buf)
    }
}

unsafe impl IoBufMut for RioSlice {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.buf.stable_mut_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_registers_and_hands_out_disjoint_slices() {
        let pool = RioBufferPool::new(256, 4).expect("registration");
        assert_eq!(pool.available(), 4);

        let mut a = pool.acquire().unwrap();
        let mut b = pool.acquire().unwrap();
        assert_eq!(pool.available(), 2);

        // Distinct regions of one registration.
        let (ba, bb) = (a.rio_buf(), b.rio_buf());
        assert_eq!(ba.BufferId, bb.BufferId);
        assert_ne!(ba.Offset, bb.Offset);
        assert_eq!(ba.Length, 256);

        a.fill(0xAA);
        b.fill(0xBB);
        assert!(a.iter().all(|&x| x == 0xAA));
        assert!(b.iter().all(|&x| x == 0xBB));

        a.set_len(10);
        assert_eq!(a.rio_buf().Length, 10);
        assert_eq!(a.rio_buf().Offset, ba.Offset);
        a.reset_len();
        assert_eq!(a.len(), 256);

        drop(a);
        drop(b);
        assert_eq!(pool.available(), 4);
    }

    #[test]
    fn pool_exhaustion_is_reported_not_blocked() {
        let pool = RioBufferPool::new(64, 2).expect("registration");
        let _x = pool.acquire().unwrap();
        let _y = pool.acquire().unwrap();
        assert!(pool.acquire().is_none(), "exhaustion must not block");
    }

    #[test]
    fn slice_outlives_pool_handle() {
        // The registration must survive a dropped pool handle: an in-flight
        // RIO op could still reference it, and there is no way to cancel one.
        let mut slice = {
            let pool = RioBufferPool::new(128, 1).expect("registration");
            let s = pool.acquire().unwrap();
            drop(pool);
            s
        };
        slice.fill(0x5A);
        assert!(slice.iter().all(|&x| x == 0x5A));
        assert_eq!(slice.rio_buf().Length, 128);
    }
}
