//! Page-aligned buffers for unbuffered scatter/gather I/O.

use super::*;

/// A set of `pages` page-sized, page-aligned buffers (one `VirtualAlloc`
/// region), for the scatter/gather file IO. Each page becomes one
/// `FILE_SEGMENT_ELEMENT`. The whole region is addressable as one contiguous
/// byte slice for filling/inspecting.
pub struct PageBuffers {
    pub(super) region: Region,
    pub(super) pages: usize,
    pub(super) page_size: usize,
}

impl PageBuffers {
    /// Allocates `pages` page-aligned pages (must be >= 1).
    pub fn new(pages: usize) -> windows_core::Result<Self> {
        let pages = pages.max(1);
        let page_size = page_size();
        Ok(Self {
            region: Region::new(pages * page_size)?,
            pages,
            page_size,
        })
    }

    /// The number of pages.
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// The page size these buffers are aligned/sized to.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// The total byte length (`pages * page_size`).
    pub fn len(&self) -> usize {
        self.pages * self.page_size
    }

    /// Whether there are zero bytes (never true — there is always >= 1 page).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The region as one contiguous byte slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.region.base(), self.len()) }
    }

    /// The region as one contiguous mutable byte slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.region.base(), self.len()) }
    }
}

// The region is contiguous and its address never moves, so these buffers are
// an ordinary owned IO buffer as well — usable with `read_at`/`write_at`, not
// only with scatter/gather.
//
// Scatter/gather deliberately keeps taking `PageBuffers` rather than any
// `IoBufMut`: it needs each segment to be exactly one page, and this type
// *proves* that where a generic buffer could only be checked at run time.
// Unbuffered IO answers a misaligned buffer with ERROR_INVALID_PARAMETER, so
// the guarantee is worth more than the uniformity.
unsafe impl IoBuf for PageBuffers {
    fn stable_ptr(&self) -> *const u8 {
        self.region.base()
    }
    fn len(&self) -> usize {
        PageBuffers::len(self)
    }
}

unsafe impl IoBufMut for PageBuffers {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.region.base()
    }
}

/// Op-block payload for scatter/gather: owns the page buffers and the segment
/// array (built in place, read by the kernel for the op's duration).
pub(super) struct VectorPayload {
    pub(super) buffers: PageBuffers,
    pub(super) segments: Vec<FILE_SEGMENT_ELEMENT>,
}

unsafe impl Send for VectorPayload {}

impl VectorPayload {
    /// Builds the null-terminated segment array pointing at each page.
    pub(super) fn build_segments(&mut self) {
        self.segments.clear();
        for i in 0..self.buffers.pages {
            self.segments.push(FILE_SEGMENT_ELEMENT {
                Buffer: unsafe { self.buffers.region.base().add(i * self.buffers.page_size) } as *mut c_void,
            });
        }
        // NULL-terminator element.
        self.segments.push(FILE_SEGMENT_ELEMENT { Alignment: 0 });
    }
}
