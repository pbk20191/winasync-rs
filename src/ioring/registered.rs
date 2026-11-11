//! Registered files and buffers: tables the kernel addresses by index.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Files registered with a ring: see [`IoRing::register_files`].
///
/// A registered file is named to the kernel by index instead of by handle,
/// which saves looking the handle up on every operation. The ring keeps the
/// handles open itself -- measured, a registration does not: closing the
/// caller's handle after registering made the index fail with
/// `ERROR_INVALID_HANDLE`.
#[derive(Clone)]
pub struct RegisteredFiles {
    pub(super) table: Arc<FileTable>,
}

pub(super) struct FileTable {
    pub(super) ring: usize,
    pub(super) generation: u64,
    pub(super) handles: Vec<HANDLE>,
    /// Whatever keeps the handles open -- the caller's `Vec<H>`, whose type
    /// the ring cannot carry, boxed once for the table.
    pub(super) _keep: Box<dyn Send + Sync>,
}

// Handle values and the owners that keep them open; never mutated.
unsafe impl Send for FileTable {}
unsafe impl Sync for FileTable {}

impl RegisteredFiles {
    /// The `index`th registered file, as a handle operations take.
    pub fn get(&self, index: usize) -> Option<RegisteredFile> {
        (index < self.table.handles.len()).then(|| RegisteredFile {
            table: self.table.clone(),
            index: index as u32,
        })
    }

    pub fn len(&self) -> usize {
        self.table.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.handles.is_empty()
    }
}

/// One registered file; any operation that takes a [`RingHandle`] takes
/// this, and names the file by index. Valid on its own ring while its
/// registration is the active one: once [`IoRing::register_files`] replaces
/// it, operations on this fail with `E_INVALIDARG` rather than reach
/// whatever the new table has at the same index.
#[derive(Clone)]
pub struct RegisteredFile {
    pub(super) table: Arc<FileTable>,
    pub(super) index: u32,
}

unsafe impl RingHandle for RegisteredFile {
    fn raw_handle(&self) -> HANDLE {
        self.table.handles[self.index as usize]
    }

    fn ring_registration(&self) -> Option<FileRegistration> {
        Some(FileRegistration {
            ring: self.table.ring,
            generation: self.table.generation,
            index: self.index,
        })
    }
}

/// Registers `files` as one table: see [`IoRing::register_files`].
pub struct RegisterFilesOp {
    pub(super) table: Arc<FileTable>,
}

impl RingOperation for RegisterFilesOp {
    type Output = Result<RegisteredFiles>;

    fn file(&self) -> IORING_HANDLE_REF {
        IORING_HANDLE_REF::default()
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        // One replacement at a time: a second would race the first's
        // confirmation.
        if ctx.ring.files.pending.is_some() {
            return Err(BuildStop::Wait);
        }
        let table = self.table.clone();
        ctx.entry(|h, user_data| unsafe { BuildIoRingRegisterFileHandles(h, &table.handles, user_data) })?;
        ctx.ring.files.pending = Some(PendingRegistration {
            user_data: ctx.user_data,
            generation: self.table.generation,
            table: self.table.clone(),
        });
        Ok(())
    }

    fn finish(self, result: HRESULT, _information: usize) -> Self::Output {
        result.ok().map(|()| RegisteredFiles { table: self.table })
    }

    fn fail(self, error: Error) -> Self::Output {
        Err(error)
    }
}

/// Buffers registered with a ring: see [`IoRing::register_buffers`].
///
/// Registered memory is named to the kernel by index and offset instead of
/// by address. Each buffer is handed out whole, one holder at a time, as a
/// [`RingBuffer`] -- which is how the registered memory stays out of two
/// writers' hands -- and comes back when that is dropped.
#[derive(Clone)]
pub struct RegisteredBuffers {
    pub(super) table: Arc<BufferTable>,
}

pub(super) struct BufferTable {
    pub(super) ring: usize,
    pub(super) generation: u64,
    pub(super) infos: Vec<IORING_BUFFER_INFO>,
    /// The owners of the memory -- the caller's `Vec<B>`, or a slab's
    /// region -- boxed once for the table; only ever dropped.
    pub(super) _keep: Box<dyn Send>,
    /// One flag per buffer, `true` while it is free. A buffer travels with
    /// the operation's result and may be dropped on another thread, so this
    /// is shared state; flags rather than a list keep it lock-free.
    pub(super) free: Vec<AtomicBool>,
}

// The memory is reached only through `RingBuffer`s, one per buffer at a
// time; the owners are never touched again until dropped.
unsafe impl Send for BufferTable {}
unsafe impl Sync for BufferTable {}

impl BufferTable {
    /// Claims buffer `index` if it is free.
    fn claim(&self, index: usize) -> bool {
        self.free
            .get(index)
            .is_some_and(|f| f.compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire).is_ok())
    }
}

impl RegisteredBuffers {
    /// Any free buffer, or `None` when every one is out.
    pub fn take(&self) -> Option<RingBuffer> {
        let index = (0..self.table.free.len()).find(|&i| self.table.claim(i))?;
        Some(RingBuffer {
            table: self.table.clone(),
            index: index as u32,
            len: 0,
        })
    }

    /// The `index`th buffer, if it is free.
    pub fn take_index(&self, index: usize) -> Option<RingBuffer> {
        self.table.claim(index).then(|| RingBuffer {
            table: self.table.clone(),
            index: index as u32,
            len: 0,
        })
    }

    /// How many buffers were registered.
    pub fn len(&self) -> usize {
        self.table.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.infos.is_empty()
    }

    /// How many are free right now.
    pub fn available(&self) -> usize {
        self.table.free.iter().filter(|f| f.load(Ordering::Acquire)).count()
    }
}

/// One registered buffer, held exclusively until dropped. An [`IoBuf`] and
/// [`IoBufMut`] like any other -- reads and writes on its ring address it by
/// index, everything else by its address -- and so is a
/// [`Slice`](crate::common::buffer::Slice) of it.
///
/// Starts empty: `len` is what holds data, and a read sets it.
pub struct RingBuffer {
    pub(super) table: Arc<BufferTable>,
    pub(super) index: u32,
    pub(super) len: usize,
}

impl RingBuffer {
    /// Sets how much of the buffer holds data, up to its size: before a
    /// write of what was filled in by hand.
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(self.capacity());
    }

    /// The buffer's registered size.
    pub fn capacity(&self) -> usize {
        self.table.infos[self.index as usize].Length as usize
    }

    pub(super) fn base(&self) -> *mut u8 {
        self.table.infos[self.index as usize].Address.cast()
    }
}

impl core::ops::Deref for RingBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.base(), self.len) }
    }
}

impl core::ops::DerefMut for RingBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.base(), self.len) }
    }
}

impl Drop for RingBuffer {
    fn drop(&mut self) {
        self.table.free[self.index as usize].store(true, Ordering::Release);
    }
}

unsafe impl IoBuf for RingBuffer {
    fn stable_ptr(&self) -> *const u8 {
        self.base()
    }
    fn len(&self) -> usize {
        self.len
    }
    fn bytes_total(&self) -> usize {
        self.capacity()
    }
    fn ring_registration(&self) -> Option<crate::common::buffer::RingRegistration> {
        Some(crate::common::buffer::RingRegistration {
            ring: self.table.ring,
            generation: self.table.generation,
            index: self.index,
            offset: 0,
        })
    }
}

unsafe impl IoBufMut for RingBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.base()
    }
    unsafe fn set_init(&mut self, n: usize) {
        self.len = self.len.max(n);
    }
}

/// Registers buffers as one table: see [`IoRing::register_buffers`].
pub struct RegisterBuffersOp {
    pub(super) table: Arc<BufferTable>,
}

impl RingOperation for RegisterBuffersOp {
    type Output = Result<RegisteredBuffers>;

    fn file(&self) -> IORING_HANDLE_REF {
        IORING_HANDLE_REF::default()
    }

    fn build(&mut self, ctx: &mut BuildCtx<'_>) -> core::result::Result<(), BuildStop> {
        if ctx.ring.buffers.pending.is_some() {
            return Err(BuildStop::Wait);
        }
        let table = self.table.clone();
        ctx.entry(|h, user_data| unsafe { BuildIoRingRegisterBuffers(h, &table.infos, user_data) })?;
        ctx.ring.buffers.pending = Some(PendingRegistration {
            user_data: ctx.user_data,
            generation: self.table.generation,
            table: self.table.clone(),
        });
        Ok(())
    }

    fn finish(self, result: HRESULT, _information: usize) -> Self::Output {
        result.ok().map(|()| RegisteredBuffers { table: self.table })
    }

    fn fail(self, error: Error) -> Self::Output {
        Err(error)
    }
}

impl IoRing {
    /// Registers `files`, replacing any files registered before; resolves to
    /// the table, from which [`RegisteredFile`]s are taken. If the kernel
    /// refuses it, no files are registered afterwards -- the previous table
    /// is gone too.
    ///
    /// The ring keeps a clone of each handle for as long as the table is
    /// registered or in use. Operations on files of the replaced table fail
    /// with `E_INVALIDARG` from then on; operations that need registered
    /// files wait while a registration is being confirmed. The kernel has no
    /// way to register nothing (an empty table is refused), so there is no
    /// unregister: replace, or drop the ring.
    pub fn register_files<H: RingHandle + Clone + 'static>(&self, files: &[H]) -> RingOp<RegisterFilesOp> {
        let mut ring = self.inner.ring.borrow_mut();
        let generation = ring.next_generation;
        ring.next_generation += 1;
        let table = Arc::new(FileTable {
            ring: ring.id,
            generation,
            handles: files.iter().map(RingHandle::raw_handle).collect(),
            _keep: Box::new(files.to_vec()),
        });
        drop(ring);
        RingOp::new(&self.inner, RegisterFilesOp { table }, false)
    }

    /// Registers `buffers` -- any owned buffers, each by its whole
    /// [`IoBuf::bytes_total`] -- replacing any registered before; resolves to
    /// the table, from which [`RingBuffer`]s are taken.
    ///
    /// The memory stays registered, and owned by the table, until the table
    /// is replaced and every `RingBuffer` and operation using it is gone.
    /// Fails with `E_INVALIDARG` for a buffer over 4 GiB, which a
    /// registration cannot describe.
    pub fn register_buffers<B: IoBufMut + 'static>(&self, mut buffers: Vec<B>) -> RingOp<RegisterBuffersOp> {
        let mut infos = Vec::with_capacity(buffers.len());
        let mut too_big = false;
        for b in buffers.iter_mut() {
            let Ok(len) = u32::try_from(b.bytes_total()) else {
                too_big = true;
                break;
            };
            infos.push(IORING_BUFFER_INFO {
                Address: b.stable_mut_ptr().cast(),
                Length: len,
            });
        }
        self.register_infos(infos, Box::new(buffers), too_big)
    }

    /// Registers one region of `count` buffers of `size` bytes each -- a slab
    /// carved from a single page-aligned allocation.
    pub fn register_slab(&self, size: u32, count: u32) -> Result<RingOp<RegisterBuffersOp>> {
        let total = (size as usize)
            .checked_mul(count as usize)
            .ok_or_else(|| Error::new(windows::Win32::Foundation::E_INVALIDARG, "slab too large"))?;
        let region = crate::common::buffer::Region::new(total.max(1))?;
        let infos = (0..count)
            .map(|i| IORING_BUFFER_INFO {
                Address: unsafe { region.base().add(i as usize * size as usize) }.cast(),
                Length: size,
            })
            .collect();
        Ok(self.register_infos(infos, Box::new(region), false))
    }

    pub(super) fn register_infos(
        &self,
        infos: Vec<IORING_BUFFER_INFO>,
        keep: Box<dyn Send>,
        too_big: bool,
    ) -> RingOp<RegisterBuffersOp> {
        let mut ring = self.inner.ring.borrow_mut();
        let generation = ring.next_generation;
        ring.next_generation += 1;
        let count = infos.len();
        let table = Arc::new(BufferTable {
            ring: ring.id,
            generation,
            infos,
            _keep: keep,
            free: (0..count).map(|_| AtomicBool::new(true)).collect(),
        });
        drop(ring);
        if too_big {
            return RingOp::failed(
                &self.inner,
                RegisterBuffersOp { table },
                Error::new(windows::Win32::Foundation::E_INVALIDARG, "a buffer over 4 GiB"),
            );
        }
        RingOp::new(&self.inner, RegisterBuffersOp { table }, false)
    }
}

// ---------------------------------------------------------------------------
// The pump
// ---------------------------------------------------------------------------
