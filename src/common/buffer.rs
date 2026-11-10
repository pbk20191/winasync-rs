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
//! # What can be a buffer
//!
//! Anything owned whose bytes stay put: `Vec<u8>` and `Box<[u8]>`;
//! `Arc<[u8]>` / `Arc<Vec<u8>>` to send one allocation on many sockets;
//! `&'static` memory; [`SlabBuf`]; `bytes::Bytes` / `BytesMut` with the
//! `bytes` feature; and, for memory this crate cannot see the owner of,
//! [`RawBuf`]. [`IoBuf::slice`] narrows any of them to a range.
//!
//! A read may fill [`IoBuf::bytes_total`] -- for a `Vec`, its capacity -- so
//! `Vec::with_capacity(n)` is a read buffer that was never zeroed, and its
//! length is set to what arrived ([`IoBufMut::set_init`]).
//!
//! A borrowed `&mut [u8]` is a buffer too, but only for the `unsafe`
//! `*_unchecked` operations. Dropping a future can cancel the I/O, but
//! forgetting one is safe Rust and cancels nothing, so a borrow could end
//! while the kernel still writes through it; those operations make the
//! caller promise not to. See [`IoBuf`]'s "Borrowed buffers". Without the
//! promise, read into an owned buffer and copy -- what `tp::io::async_rw`
//! does for `futures-io`.
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
/// Two sizes, because a buffer is used two ways. A write sends
/// [`len`](Self::len) bytes -- the ones that hold data. A read may fill
/// [`bytes_total`](Self::bytes_total) -- all the room there is, which for a
/// `Vec` includes capacity beyond its length, so a read needs no zeroed
/// buffer to land in. [`IoBufMut::set_init`] then says how much it filled.
///
/// # Borrowed buffers
///
/// The trait does not require `'static`, so `&'a mut [u8]` is a buffer too.
/// Every safe operation in this crate asks for `B: IoBuf + 'static` -- an
/// owned buffer moves into the operation and outlives it whatever happens to
/// the future. A borrowed one does not: a future can be forgotten, which is
/// safe and cancels nothing, and the borrow would end while the kernel still
/// writes through it. So borrowed buffers go only to the `*_unchecked`
/// operations, which are `unsafe` for exactly that, and whose futures wait
/// for the kernel when dropped.
///
/// # Safety
/// [`stable_ptr`](Self::stable_ptr) must keep returning the same address, and
/// that address must stay valid and allocated, for as long as the value
/// exists — including after the value is moved. Implementors that store the
/// bytes inline (an array by value, say) do not qualify.
///
/// [`len`](Self::len) bytes starting there must be readable, and
/// [`bytes_total`](Self::bytes_total) must be at least `len`.
pub unsafe trait IoBuf: Send {
    /// The address the kernel will read from or write to.
    fn stable_ptr(&self) -> *const u8;

    /// How many bytes hold data: what a write sends.
    fn len(&self) -> usize;

    /// How many bytes a read may fill, starting at
    /// [`stable_ptr`](Self::stable_ptr). [`len`](Self::len) unless the
    /// buffer has room past its data.
    fn bytes_total(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A view of `range` of this buffer, which an operation takes in its
    /// place and hands back whole: read into the second half of a `Vec`, or
    /// send a frame out of a larger one, without splitting or copying it.
    ///
    /// `range` is in bytes from the start and may reach past `len` up to
    /// `bytes_total`, so a read can land in spare capacity.
    ///
    /// # Panics
    /// If `range` is reversed or ends past `bytes_total`.
    fn slice(self, range: impl core::ops::RangeBounds<usize>) -> Slice<Self>
    where
        Self: Sized,
    {
        use core::ops::Bound;
        let begin = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&n) => n + 1,
            Bound::Excluded(&n) => n,
            Bound::Unbounded => self.bytes_total(),
        };
        assert!(
            begin <= end && end <= self.bytes_total(),
            "slice {begin}..{end} out of a buffer of {}",
            self.bytes_total()
        );
        Slice {
            buf: self,
            begin,
            end,
        }
    }
}

/// An [`IoBuf`] the kernel may write into.
///
/// # Safety
/// The same address as [`IoBuf::stable_ptr`], and
/// [`IoBuf::bytes_total`] bytes there must be writable.
pub unsafe trait IoBufMut: IoBuf {
    fn stable_mut_ptr(&mut self) -> *mut u8;

    /// Called after a read wrote `n` bytes from the start. A buffer that
    /// tracks how much of it holds data -- a `Vec`, whose length grows to
    /// cover them -- updates that here; a fixed one need not do anything.
    ///
    /// # Safety
    /// `n` bytes from [`stable_mut_ptr`](Self::stable_mut_ptr) have been
    /// written, and `n <= bytes_total()`.
    unsafe fn set_init(&mut self, n: usize) {
        let _ = n;
    }
}

// Moving a Vec moves the pointer/len/cap triple, never the heap allocation.
unsafe impl IoBuf for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
    /// The whole allocation: a read into `Vec::with_capacity(n)` fills it
    /// without the bytes being zeroed first.
    fn bytes_total(&self) -> usize {
        self.capacity()
    }
}

unsafe impl IoBufMut for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
    /// Grows the length to cover what was read, never shrinks it: a
    /// `vec![0; n]` keeps its length, as it always has, and one with spare
    /// capacity ends exactly where the data does.
    unsafe fn set_init(&mut self, n: usize) {
        if n > self.len() {
            unsafe { self.set_len(n) };
        }
    }
}

/// A range of an owned buffer; see [`IoBuf::slice`].
///
/// An operation on it uses only `begin..end` and hands the whole `Slice`
/// back; [`into_inner`](Self::into_inner) recovers the buffer.
pub struct Slice<B> {
    buf: B,
    begin: usize,
    end: usize,
}

impl<B> Slice<B> {
    /// Where the range starts in the underlying buffer.
    pub fn begin(&self) -> usize {
        self.begin
    }

    /// Where the range ends in the underlying buffer.
    pub fn end(&self) -> usize {
        self.end
    }

    pub fn get_ref(&self) -> &B {
        &self.buf
    }

    /// The whole buffer back, with whatever a read wrote into the range.
    pub fn into_inner(self) -> B {
        self.buf
    }
}

unsafe impl<B: IoBuf> IoBuf for Slice<B> {
    fn stable_ptr(&self) -> *const u8 {
        unsafe { self.buf.stable_ptr().add(self.begin) }
    }
    /// The part of the range that holds data.
    fn len(&self) -> usize {
        self.buf.len().min(self.end).saturating_sub(self.begin)
    }
    fn bytes_total(&self) -> usize {
        self.end - self.begin
    }
}

unsafe impl<B: IoBufMut> IoBufMut for Slice<B> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        unsafe { self.buf.stable_mut_ptr().add(self.begin) }
    }
    unsafe fn set_init(&mut self, n: usize) {
        unsafe { self.buf.set_init(self.begin + n) };
    }
}

/// Read-only and shared: the kernel only reads what it sends, and so does
/// every other holder, so one allocation can go out on many sockets at once.
unsafe impl IoBuf for std::sync::Arc<[u8]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
}

/// As `Arc<[u8]>`. Nobody can reach the `Vec` mutably while the operation
/// holds a clone -- `Arc::get_mut` needs the only one -- so its allocation
/// cannot move.
unsafe impl IoBuf for std::sync::Arc<Vec<u8>> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
}

/// A borrow. With `'static` -- a `static`, a leaked allocation -- any
/// operation takes it; with a shorter lifetime, only the `unsafe`
/// `*_unchecked` ones (see the trait docs).
unsafe impl IoBuf for &[u8] {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
}

unsafe impl IoBuf for &mut [u8] {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
}

unsafe impl IoBufMut for &mut [u8] {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

/// Memory this crate cannot see the owner of -- a mapped section, a C
/// library's allocation, device memory -- with whatever keeps it alive.
///
/// The type exists so such memory does not need an `IoBuf` implementation of
/// its own: the contract is stated once, at [`from_raw_parts`](Self::from_raw_parts).
pub struct RawBuf {
    ptr: *mut u8,
    len: usize,
    total: usize,
    _keeper: Box<dyn core::any::Any + Send>,
}

// The constructor's contract makes the memory usable from any thread.
unsafe impl Send for RawBuf {}

impl RawBuf {
    /// Wraps `total` bytes at `ptr`, of which the first `len` hold data.
    /// `keeper` is dropped with the `RawBuf` -- the mapping, the handle, the
    /// allocation that owns the memory.
    ///
    /// # Safety
    /// - The `total` bytes at `ptr` stay allocated, readable and writable,
    ///   and are not moved, until `keeper` is dropped.
    /// - Nothing else reads or writes them while an operation holds this
    ///   `RawBuf` -- the kernel may be writing into them.
    /// - They may be used from any thread.
    /// - `len <= total`.
    pub unsafe fn from_raw_parts(
        ptr: *mut u8,
        len: usize,
        total: usize,
        keeper: impl core::any::Any + Send,
    ) -> Self {
        assert!(len <= total, "len {len} > total {total}");
        Self {
            ptr,
            len,
            total,
            _keeper: Box::new(keeper),
        }
    }

    /// The bytes that hold data.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Sets how many bytes hold data, up to the total: before a write.
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(self.total);
    }
}

unsafe impl IoBuf for RawBuf {
    fn stable_ptr(&self) -> *const u8 {
        self.ptr
    }
    fn len(&self) -> usize {
        self.len
    }
    fn bytes_total(&self) -> usize {
        self.total
    }
}

unsafe impl IoBufMut for RawBuf {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
    unsafe fn set_init(&mut self, n: usize) {
        self.len = self.len.max(n);
    }
}

#[cfg(feature = "bytes")]
mod bytes_impls {
    use super::{IoBuf, IoBufMut};

    /// Moving a `Bytes` moves a handle, never the bytes, and it is read-only.
    unsafe impl IoBuf for bytes::Bytes {
        fn stable_ptr(&self) -> *const u8 {
            self.as_ptr()
        }
        fn len(&self) -> usize {
            bytes::Bytes::len(self)
        }
    }

    /// A `BytesMut` is unique while the operation holds it, so nothing can
    /// make it reallocate; reads fill its spare capacity, as with `Vec`.
    unsafe impl IoBuf for bytes::BytesMut {
        fn stable_ptr(&self) -> *const u8 {
            self.as_ptr()
        }
        fn len(&self) -> usize {
            bytes::BytesMut::len(self)
        }
        fn bytes_total(&self) -> usize {
            self.capacity()
        }
    }

    unsafe impl IoBufMut for bytes::BytesMut {
        fn stable_mut_ptr(&mut self) -> *mut u8 {
            self.as_mut_ptr()
        }
        unsafe fn set_init(&mut self, n: usize) {
            if n > self.len() {
                unsafe { self.set_len(n) };
            }
        }
    }
}

/// Tells `buf` how much a read wrote, if it succeeded. `n` is the read's byte
/// count, or `None` for a failed one.
pub(crate) fn mark_read<B: IoBufMut>(n: Option<usize>, mut buf: B) -> B {
    if let Some(n) = n {
        // The kernel never reports more than it was given room for.
        unsafe { buf.set_init(n.min(buf.bytes_total())) };
    }
    buf
}

/// [`mark_read`] for a scatter read: the bytes fill the buffers in order.
pub(crate) fn mark_read_vectored<B: IoBufMut>(n: Option<usize>, mut bufs: Vec<B>) -> Vec<B> {
    if let Some(mut n) = n {
        for b in bufs.iter_mut() {
            let here = n.min(b.bytes_total());
            unsafe { b.set_init(here) };
            n -= here;
        }
    }
    bufs
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

/// The system page size, cached.
pub fn page_size() -> usize {
    use std::sync::OnceLock;
    use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    static PAGE: OnceLock<usize> = OnceLock::new();
    *PAGE.get_or_init(|| {
        let mut si = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut si) };
        si.dwPageSize as usize
    })
}

/// One `VirtualAlloc` reservation, released when dropped.
///
/// The shared piece under every buffer type here: a page-aligned region whose
/// address never moves. What differs between them is how the region is carved
/// up — into interchangeable slices ([`SlabPool`]), or into fixed pages that a
/// scatter/gather call describes individually — not how it is obtained.
pub struct Region {
    base: *mut u8,
    len: usize,
}

unsafe impl Send for Region {}
unsafe impl Sync for Region {}

impl Region {
    /// Reserves and commits `len` bytes. `VirtualAlloc` returns page-aligned
    /// memory, which is what the unbuffered file paths require.
    pub fn new(len: usize) -> windows_core::Result<Self> {
        assert!(len > 0, "empty region");
        let base =
            unsafe { VirtualAlloc(None, len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) } as *mut u8;
        if base.is_null() {
            return Err(windows_core::Error::from_thread());
        }
        Ok(Self { base, len })
    }

    pub fn base(&self) -> *mut u8 {
        self.base
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFree(self.base as *mut _, 0, MEM_RELEASE);
        }
    }
}

/// A reserved region cut into fixed-size slices.
///
/// One allocation instead of one per operation, and slices return themselves
/// when dropped. `rio` builds its registered pool on this, adding the
/// `RIORegisterBuffer` call and the per-slice `RIO_BUF` descriptor; the
/// slab arithmetic is the same either way.
pub struct SlabPool {
    region: Region,
    slice_size: usize,
    free: Mutex<Vec<u32>>,
}

impl SlabPool {
    /// Reserves `slice_count` slices of `slice_size` bytes as one
    /// page-aligned `VirtualAlloc` region.
    pub fn new(slice_size: usize, slice_count: u32) -> windows_core::Result<Arc<Self>> {
        assert!(slice_size > 0 && slice_count > 0, "empty pool");
        let len = slice_size
            .checked_mul(slice_count as usize)
            .expect("pool size overflows usize");
        Ok(Arc::new(Self {
            region: Region::new(len)?,
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
        (self.region.base(), self.region.len())
    }

    fn release(&self, index: u32) {
        self.free.lock().unwrap().push(index);
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
        unsafe { core::slice::from_raw_parts(self.pool.region.base().add(self.offset()), self.len) }
    }
}

impl core::ops::DerefMut for SlabBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pool.region.base().add(self.offset()), self.len) }
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
        unsafe { self.pool.region.base().add(self.offset()) }
    }
    fn len(&self) -> usize {
        self.len
    }
}

unsafe impl IoBufMut for SlabBuf {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        unsafe { self.pool.region.base().add(self.offset()) }
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
