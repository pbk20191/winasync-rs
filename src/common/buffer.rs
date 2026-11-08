//! Buffers for completion-based IO, and a slab pool that hands them out.
//!
//! # Why ownership, not borrowing
//!
//! Every engine here is completion-based: the kernel keeps writing into a
//! buffer after the call that started the operation has returned. A `&mut
//! [u8]` cannot express that — the borrow would have to outlive the call, and
//! dropping the future would not stop the kernel. So an operation *takes* its
//! buffer and hands it back with the result, and the buffer must keep the same
//! address throughout.
//!
//! That is what [`IoBuf`] states: a pointer that stays valid while the value
//! is moved into the operation's heap block. `Vec<u8>` qualifies — moving a
//! `Vec` moves three words, never its allocation — which is why it has been
//! the de facto buffer type here, and why existing callers keep working
//! unchanged.
//!
//! The same problem produces the same shape elsewhere: `tokio-uring` splits
//! `IoBuf`/`IoBufMut` for io_uring, which is completion-based for the same
//! reason. This is that design, not a local invention.
//!
//! # What this does and does not buy
//!
//! It lets a caller bring **its own memory** — an arena, a bump allocator,
//! shared memory, pages it pinned itself — instead of being forced through
//! `Vec<u8>`. That is the point.
//!
//! It is **not** a throughput feature. Measured on this machine (see
//! `rio::bench`), recycling buffers instead of allocating per operation moved
//! 64-byte datagram throughput by less than run-to-run noise. Reach for
//! [`SlabPool`] when the allocation pattern matters for a reason you have
//! measured, not on the assumption that pooling is faster.
//!
//! # The pools that cannot be replaced
//!
//! Two buffer kinds here have allocation requirements a general pool cannot
//! satisfy, so they keep their own types and merely *implement* these traits:
//!
//! * `rio::RioSlice` must live inside a slab registered with
//!   `RIORegisterBuffer`; arbitrary memory is rejected.
//! * `tp::io::file::PageBuffers` must be page-aligned for
//!   `FILE_FLAG_NO_BUFFERING` scatter/gather.
//!
//! The asymmetry is real, and these traits leave it visible rather than
//! papering over it: a RIO operation still takes a `RioSlice` specifically.

use std::sync::{Arc, Mutex};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};

/// A buffer an operation can take ownership of.
///
/// # Safety
/// [`stable_ptr`](Self::stable_ptr) must keep returning the same address, and
/// that address must stay valid and allocated, for as long as the value
/// exists — including after the value is moved. Implementors that store the
/// bytes inline (an array by value, say) do not qualify.
///
/// [`len`](Self::len) bytes starting there must be readable.
pub unsafe trait IoBuf: Send + 'static {
    /// The address the kernel will read from or write to.
    fn stable_ptr(&self) -> *const u8;

    /// How many bytes the operation should use: the length to send, or the
    /// room available to receive into.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An [`IoBuf`] the kernel may write into.
///
/// # Safety
/// The same address as [`IoBuf::stable_ptr`], and [`IoBuf::len`] bytes there
/// must be writable.
pub unsafe trait IoBufMut: IoBuf {
    fn stable_mut_ptr(&mut self) -> *mut u8;
}

// Moving a Vec moves the pointer/len/cap triple, never the heap allocation.
unsafe impl IoBuf for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
}

unsafe impl IoBufMut for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

unsafe impl IoBuf for Box<[u8]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
}

unsafe impl IoBufMut for Box<[u8]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

/// A reserved region cut into fixed-size slices.
///
/// One allocation instead of one per operation, and slices return themselves
/// when dropped. `rio` builds its registered pool on this, adding the
/// `RIORegisterBuffer` call and the per-slice `RIO_BUF` descriptor; the
/// slab arithmetic is the same either way.
pub struct SlabPool {
    base: *mut u8,
    len: usize,
    slice_size: usize,
    free: Mutex<Vec<u32>>,
}

// The slab is plain memory this pool owns; handing slices between threads is
// the point.
unsafe impl Send for SlabPool {}
unsafe impl Sync for SlabPool {}

impl SlabPool {
    /// Reserves `slice_count` slices of `slice_size` bytes as one
    /// page-aligned `VirtualAlloc` region.
    pub fn new(slice_size: usize, slice_count: u32) -> windows_core::Result<Arc<Self>> {
        assert!(slice_size > 0 && slice_count > 0, "empty pool");
        let len = slice_size
            .checked_mul(slice_count as usize)
            .expect("pool size overflows usize");
        let base =
            unsafe { VirtualAlloc(None, len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) } as *mut u8;
        if base.is_null() {
            return Err(windows_core::Error::from_thread());
        }
        Ok(Arc::new(Self {
            base,
            len,
            slice_size,
            free: Mutex::new((0..slice_count).rev().collect()),
        }))
    }

    /// Takes a slice, or `None` when every slice is out.
    ///
    /// Non-blocking: a pool that waited would hide the backpressure its
    /// emptiness is reporting.
    pub fn acquire(self: &Arc<Self>) -> Option<SlabBuf> {
        let index = self.free.lock().unwrap().pop()?;
        Some(SlabBuf {
            pool: self.clone(),
            index,
            len: self.slice_size,
        })
    }

    pub fn available(&self) -> usize {
        self.free.lock().unwrap().len()
    }

    pub fn slice_size(&self) -> usize {
        self.slice_size
    }

    /// The whole region, for an engine that must register or pin it.
    pub fn region(&self) -> (*mut u8, usize) {
        (self.base, self.len)
    }

    fn release(&self, index: u32) {
        self.free.lock().unwrap().push(index);
    }
}

impl Drop for SlabPool {
    /// Reaching here means no [`SlabBuf`] is alive — each holds an `Arc` of
    /// this pool — so no operation can still be pointing into the region.
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFree(self.base as *mut _, 0, MEM_RELEASE);
        }
    }
}

/// One slice of a [`SlabPool`], returned when dropped.
pub struct SlabBuf {
    pool: Arc<SlabPool>,
    index: u32,
    len: usize,
}

impl SlabBuf {
    /// Byte offset of this slice within the pool's region — what an engine
    /// that registered the whole region needs to describe the slice.
    pub fn offset(&self) -> usize {
        self.index as usize * self.pool.slice_size
    }

    /// Narrows the slice to its first `len` bytes: how a send says how much
    /// is payload. Capped at the slice size.
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(self.pool.slice_size);
    }

    /// Restores the slice to its full size, for reuse as a receive buffer.
    pub fn reset_len(&mut self) {
        self.len = self.pool.slice_size;
    }

    pub fn pool(&self) -> &Arc<SlabPool> {
        &self.pool
    }
}

impl core::ops::Deref for SlabBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.pool.base.add(self.offset()), self.len) }
    }
}

impl core::ops::DerefMut for SlabBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pool.base.add(self.offset()), self.len) }
    }
}

impl Drop for SlabBuf {
    fn drop(&mut self) {
        self.pool.release(self.index);
    }
}

// The slice is an index into a pool that is itself Send + Sync, and the
// region never moves.
unsafe impl Send for SlabBuf {}
unsafe impl Sync for SlabBuf {}

unsafe impl IoBuf for SlabBuf {
    fn stable_ptr(&self) -> *const u8 {
        unsafe { self.pool.base.add(self.offset()) }
    }
    fn len(&self) -> usize {
        self.len
    }
}

unsafe impl IoBufMut for SlabBuf {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        unsafe { self.pool.base.add(self.offset()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slab_hands_out_disjoint_slices_and_takes_them_back() {
        let pool = SlabPool::new(128, 3).unwrap();
        assert_eq!(pool.available(), 3);

        let mut a = pool.acquire().unwrap();
        let mut b = pool.acquire().unwrap();
        assert_ne!(a.offset(), b.offset());

        a.fill(0x11);
        b.fill(0x22);
        assert!(a.iter().all(|&x| x == 0x11));
        assert!(b.iter().all(|&x| x == 0x22));

        // Narrowing changes the transmitted length, not the address.
        let ptr = a.stable_ptr();
        a.set_len(4);
        assert_eq!(IoBuf::len(&a), 4);
        assert_eq!(a.stable_ptr(), ptr);

        drop(a);
        drop(b);
        assert_eq!(pool.available(), 3);
        assert!(pool.acquire().is_some());
    }

    #[test]
    fn exhaustion_is_reported_not_blocked() {
        let pool = SlabPool::new(32, 1).unwrap();
        let _held = pool.acquire().unwrap();
        assert!(pool.acquire().is_none());
    }

    #[test]
    fn slice_keeps_the_region_alive_past_the_pool_handle() {
        let mut buf = {
            let pool = SlabPool::new(64, 1).unwrap();
            let b = pool.acquire().unwrap();
            drop(pool);
            b
        };
        buf.fill(0x5A);
        assert!(buf.iter().all(|&x| x == 0x5A));
    }

    #[test]
    fn vec_is_a_valid_io_buffer_across_a_move() {
        let v = vec![1u8, 2, 3];
        let before = v.stable_ptr();
        let moved = v;
        assert_eq!(moved.stable_ptr(), before, "moving a Vec never moves its allocation");
        assert_eq!(IoBuf::len(&moved), 3);
    }
}
