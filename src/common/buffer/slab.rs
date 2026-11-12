//! A page-aligned region, and a pool that hands out fixed slices of one.

use super::*;

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
    pub(super) base: *mut u8,
    pub(super) len: usize,
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
    pub(super) region: Region,
    pub(super) slice_size: usize,
    pub(super) free: Mutex<Vec<u32>>,
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

    pub(super) fn release(&self, index: u32) {
        self.free.lock().unwrap().push(index);
    }
}

/// One slice of a [`SlabPool`], returned when dropped.
pub struct SlabBuf {
    pub(super) pool: Arc<SlabPool>,
    pub(super) index: u32,
    pub(super) len: usize,
}

impl SlabBuf {
    /// Byte offset of this slice within the pool's region — what an engine
    /// that registered the whole region needs to describe the slice.
    pub fn offset(&self) -> usize {
        self.index as usize * self.pool.slice_size
    }

    /// Which slice of the pool this is, `0..slice_count` — stable for the
    /// slice's life, so an engine can keep per-slice state in a table of
    /// its own.
    pub fn index(&self) -> u32 {
        self.index
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
