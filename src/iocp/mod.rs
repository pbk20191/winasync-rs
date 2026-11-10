//! I/O completion ports, and waits that complete into them.
//!
//! [`CompletionPort`] is the port itself: create, associate handles, post,
//! and dequeue one or many completions. It is for callers who run their own
//! completion loop -- an existing reactor, a game loop, the port `rio`'s
//! `RioCq::with_iocp` notifies -- rather than `tp`'s pool.
//!
//! [`WaitPacket`] makes *any waitable object* -- an event, a semaphore, a
//! thread, a process -- deliver to a port when it is signalled, exactly like
//! an I/O completion. One loop can then wait for I/O and for objects alike,
//! with no thread parked in `WaitForMultipleObjects` and no 64-handle limit.
//! It is built on `NtCreateWaitCompletionPacket`,
//! `NtAssociateWaitCompletionPacket` and `NtCancelWaitCompletionPacket`:
//! exported by `ntdll` since Windows 8 and what the thread pool's own
//! `TP_WAIT` uses, but undocumented, so they are looked up by name when
//! first needed and their absence is an error rather than a load failure.
//! (Prototypes as in <https://github.com/tringi/win32-iocp-events>.)
//!
//! A wait packet is one-shot: once it has delivered, it stays quiet until it
//! is associated again. Mutexes are not supported -- a wait packet does not
//! acquire what it waits on.

use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::sync::OnceLock;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatusEx, PostQueuedCompletionStatus, OVERLAPPED,
    OVERLAPPED_ENTRY,
};
use windows_core::{Error, HRESULT, Result};

/// One completion, as a port delivers it.
///
/// `overlapped` is the `OVERLAPPED` pointer as an address: for a completion
/// of real I/O it points at the caller's `OVERLAPPED`; for a posted
/// completion or a [`WaitPacket`] it is whatever value was given, and the
/// kernel never dereferences it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub key: usize,
    pub overlapped: usize,
    pub bytes: u32,
    /// The NTSTATUS the completion carries: an I/O's result, or what a
    /// [`WaitPacket`] was associated with. 0 for a posted completion.
    pub status: i32,
}

impl From<&OVERLAPPED_ENTRY> for Completion {
    fn from(e: &OVERLAPPED_ENTRY) -> Self {
        Completion {
            key: e.lpCompletionKey,
            overlapped: e.lpOverlapped as usize,
            bytes: e.dwNumberOfBytesTransferred,
            status: e.Internal as i32,
        }
    }
}

/// An I/O completion port.
pub struct CompletionPort {
    handle: OwnedHandle,
}

impl CompletionPort {
    /// A new port. `concurrency` is how many threads the port lets run at
    /// once; 0 means one per processor.
    pub fn new(concurrency: u32) -> Result<CompletionPort> {
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, concurrency)? };
        Ok(CompletionPort {
            handle: unsafe { OwnedHandle::from_raw_handle(port.0) },
        })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    /// Associates `handle` -- opened for overlapped I/O -- so its
    /// completions arrive here with `key`. A handle can be associated with
    /// one port, for good.
    pub fn associate(&self, handle: BorrowedHandle<'_>, key: usize) -> Result<()> {
        unsafe { CreateIoCompletionPort(HANDLE(handle.as_raw_handle()), Some(self.raw()), key, 0)? };
        Ok(())
    }

    /// Queues `completion` as if an operation had finished. Its `status` is
    /// not carried: a posted completion always reads as 0.
    pub fn post(&self, completion: Completion) -> Result<()> {
        unsafe {
            PostQueuedCompletionStatus(
                self.raw(),
                completion.bytes,
                completion.key,
                Some(completion.overlapped as *const OVERLAPPED),
            )
        }
    }

    /// Waits up to `timeout` (`None`: forever) for one completion; `None`
    /// if none came.
    pub fn dequeue(&self, timeout: Option<Duration>) -> Result<Option<Completion>> {
        let mut out = Vec::with_capacity(1);
        self.dequeue_many(&mut out, 1, timeout)?;
        Ok(out.pop())
    }

    /// Waits up to `timeout` for completions and appends up to `max` of
    /// them to `out` in one call; returns how many. Zero means the timeout
    /// ran out.
    pub fn dequeue_many(&self, out: &mut Vec<Completion>, max: usize, timeout: Option<Duration>) -> Result<usize> {
        let mut entries = vec![OVERLAPPED_ENTRY::default(); max.max(1)];
        let mut removed = 0u32;
        let ms = match timeout {
            None => u32::MAX,
            Some(t) => t.as_millis().min(u128::from(u32::MAX - 1)) as u32,
        };
        match unsafe { GetQueuedCompletionStatusEx(self.raw(), &mut entries, &mut removed, ms, false) } {
            Ok(()) => {}
            // A timeout is the wait code, reported as a Win32 error.
            Err(e) if e.code() == windows::Win32::Foundation::WIN32_ERROR(WAIT_TIMEOUT.0).to_hresult() => {
                return Ok(0);
            }
            Err(e) => return Err(e),
        }
        out.extend(entries[..removed as usize].iter().map(Completion::from));
        Ok(removed as usize)
    }
}

impl AsHandle for CompletionPort {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }
}

// ---------------------------------------------------------------------------
// Wait completion packets
// ---------------------------------------------------------------------------

type NtCreate = unsafe extern "system" fn(*mut HANDLE, u32, *const core::ffi::c_void) -> i32;
type NtAssociate = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    HANDLE,
    *mut core::ffi::c_void,
    *mut core::ffi::c_void,
    i32,
    usize,
    *mut u8,
) -> i32;
type NtCancel = unsafe extern "system" fn(HANDLE, u8) -> i32;

struct NtApi {
    create: NtCreate,
    associate: NtAssociate,
    cancel: NtCancel,
}

/// The three exports, found once; `None` if this `ntdll` lacks them.
fn nt() -> Result<&'static NtApi> {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    static API: OnceLock<Option<NtApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let ntdll = GetModuleHandleW(windows_core::w!("ntdll.dll")).ok()?;
        let find = |name: &[u8]| GetProcAddress(ntdll, windows_core::PCSTR(name.as_ptr()));
        Some(NtApi {
            create: core::mem::transmute::<unsafe extern "system" fn() -> isize, NtCreate>(
                find(b"NtCreateWaitCompletionPacket\0")?,
            ),
            associate: core::mem::transmute::<unsafe extern "system" fn() -> isize, NtAssociate>(
                find(b"NtAssociateWaitCompletionPacket\0")?,
            ),
            cancel: core::mem::transmute::<unsafe extern "system" fn() -> isize, NtCancel>(
                find(b"NtCancelWaitCompletionPacket\0")?,
            ),
        })
    })
    .as_ref()
    .ok_or_else(|| {
        Error::new(
            windows::Win32::Foundation::ERROR_PROC_NOT_FOUND.to_hresult(),
            "this ntdll has no wait completion packets (Windows 8 or later needed)",
        )
    })
}

/// An NTSTATUS failure as the `HRESULT` that carries it (`HRESULT_FROM_NT`).
fn nt_error(status: i32) -> Error {
    Error::from_hresult(HRESULT(status | 0x1000_0000))
}

/// What [`WaitPacket::cancel`] left behind.
///
/// Two outcomes, not three, because the kernel reports two. Measured: a
/// wait cancelled before its object was signalled, and one that had fired
/// but whose completion `remove_signaled` took back off the port, both
/// return `STATUS_SUCCESS`; only a fired wait whose completion stays on the
/// port returns something else (`STATUS_PENDING`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    /// No completion from this arming will be dequeued: it never fired, or
    /// its completion was removed.
    Cancelled,
    /// It fired, and its completion is on the port (or already dequeued).
    AlreadyDelivered,
}

/// A wait on an object that completes into a port; see the module docs.
pub struct WaitPacket {
    handle: OwnedHandle,
}

impl WaitPacket {
    pub fn new() -> Result<WaitPacket> {
        let api = nt()?;
        let mut handle = HANDLE::default();
        // GENERIC_ALL, as the reference implementation asks for.
        let status = unsafe { (api.create)(&mut handle, 0x1000_0000, core::ptr::null()) };
        if status < 0 {
            return Err(nt_error(status));
        }
        Ok(WaitPacket {
            handle: unsafe { OwnedHandle::from_raw_handle(handle.0) },
        })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    /// Arms the packet: when `target` is signalled, `completion` is queued
    /// to `port`. Returns whether `target` was already signalled, in which
    /// case the completion is queued at once.
    ///
    /// `target` is an event, semaphore, thread or process. It need only be
    /// valid for this call: the wait holds the object, not the handle.
    /// Arming a packet that is already armed replaces the earlier wait.
    pub fn associate(
        &self,
        port: &CompletionPort,
        target: BorrowedHandle<'_>,
        completion: Completion,
    ) -> Result<bool> {
        self.associate_with(port.as_handle(), target, completion)
    }

    /// [`associate`](Self::associate), for a port this crate did not
    /// create.
    pub fn associate_with(
        &self,
        port: BorrowedHandle<'_>,
        target: BorrowedHandle<'_>,
        completion: Completion,
    ) -> Result<bool> {
        let api = nt()?;
        let mut already = 0u8;
        let status = unsafe {
            (api.associate)(
                self.raw(),
                HANDLE(port.as_raw_handle()),
                HANDLE(target.as_raw_handle()),
                completion.key as *mut core::ffi::c_void,
                completion.overlapped as *mut core::ffi::c_void,
                completion.status,
                completion.bytes as usize,
                &mut already,
            )
        };
        if status < 0 {
            return Err(nt_error(status));
        }
        Ok(already != 0)
    }

    /// Disarms the packet. With `remove_signaled`, a completion it has
    /// already queued and nobody has dequeued is taken back off the port.
    pub fn cancel(&self, remove_signaled: bool) -> Result<CancelOutcome> {
        const STATUS_SUCCESS: i32 = 0;
        const STATUS_PENDING: i32 = 0x0000_0103;
        let api = nt()?;
        let status = unsafe { (api.cancel)(self.raw(), remove_signaled as u8) };
        match status {
            STATUS_SUCCESS => Ok(CancelOutcome::Cancelled),
            STATUS_PENDING => Ok(CancelOutcome::AlreadyDelivered),
            s if s < 0 => Err(nt_error(s)),
            s => Err(Error::new(
                windows::Win32::Foundation::E_UNEXPECTED,
                format!("NtCancelWaitCompletionPacket returned {s:#x}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent};

    struct Event(OwnedHandle);
    impl Event {
        fn new() -> Self {
            let h = unsafe { CreateEventW(None, true, false, None).unwrap() };
            Event(unsafe { OwnedHandle::from_raw_handle(h.0) })
        }
        fn raw(&self) -> HANDLE {
            HANDLE(self.0.as_raw_handle())
        }
        fn set(&self) {
            unsafe { SetEvent(self.raw()).unwrap() };
        }
        fn reset(&self) {
            unsafe { ResetEvent(self.raw()).unwrap() };
        }
    }

    const SHORT: Option<Duration> = Some(Duration::from_millis(200));

    fn tagged(key: usize) -> Completion {
        Completion {
            key,
            overlapped: 0xABC0 + key,
            bytes: 7,
            status: 0,
        }
    }

    #[test]
    fn posts_come_back_in_bulk() {
        let port = CompletionPort::new(0).unwrap();
        for k in 1..=3 {
            port.post(tagged(k)).unwrap();
        }
        let mut got = Vec::new();
        assert_eq!(port.dequeue_many(&mut got, 8, SHORT).unwrap(), 3);
        assert_eq!(got.iter().map(|c| c.key).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(got[0], tagged(1));
        assert_eq!(port.dequeue(SHORT).unwrap(), None);
    }

    /// An event's signal arrives as a completion carrying what the packet
    /// was armed with -- once, until armed again.
    #[test]
    fn a_signalled_event_completes_once_per_arming() {
        let port = CompletionPort::new(0).unwrap();
        let event = Event::new();
        let packet = WaitPacket::new().unwrap();
        let armed = Completion { status: 0x42, ..tagged(9) };
        assert!(!packet.associate(&port, event.0.as_handle(), armed).unwrap());
        assert_eq!(port.dequeue(SHORT).unwrap(), None, "fired before the signal");
        event.set();
        assert_eq!(port.dequeue(SHORT).unwrap(), Some(armed));
        // One-shot: still signalled, but nothing more until re-armed.
        assert_eq!(port.dequeue(SHORT).unwrap(), None);
        // Re-armed on a signalled event: reported, and queued at once.
        assert!(packet.associate(&port, event.0.as_handle(), armed).unwrap());
        assert_eq!(port.dequeue(SHORT).unwrap(), Some(armed));
        event.reset();
    }

    #[test]
    fn cancelling_reports_where_the_wait_was() {
        let port = CompletionPort::new(0).unwrap();
        let event = Event::new();
        let packet = WaitPacket::new().unwrap();

        packet.associate(&port, event.0.as_handle(), tagged(1)).unwrap();
        let before = packet.cancel(true).unwrap();
        event.set();
        assert_eq!(port.dequeue(SHORT).unwrap(), None, "a cancelled wait fired");
        event.reset();

        packet.associate(&port, event.0.as_handle(), tagged(2)).unwrap();
        event.set();
        std::thread::sleep(Duration::from_millis(50));
        let removed = packet.cancel(true).unwrap();
        assert_eq!(port.dequeue(SHORT).unwrap(), None, "the removed completion stayed queued");
        event.reset();

        packet.associate(&port, event.0.as_handle(), tagged(3)).unwrap();
        event.set();
        std::thread::sleep(Duration::from_millis(50));
        let kept = packet.cancel(false).unwrap();
        assert_eq!(port.dequeue(SHORT).unwrap().map(|c| c.key), Some(3));
        eprintln!("cancel outcomes: before={before:?} removed={removed:?} kept={kept:?}");
        assert_eq!(before, CancelOutcome::Cancelled);
        assert_eq!(removed, CancelOutcome::Cancelled);
        assert_eq!(kept, CancelOutcome::AlreadyDelivered);
    }

    /// The wait holds the object, so the caller's handle may close.
    #[test]
    fn the_target_handle_may_close_after_arming() {
        let port = CompletionPort::new(0).unwrap();
        let event = Event::new();
        let duplicate = event.0.try_clone().unwrap();
        let packet = WaitPacket::new().unwrap();
        packet.associate(&port, duplicate.as_handle(), tagged(5)).unwrap();
        drop(duplicate);
        event.set();
        assert_eq!(port.dequeue(SHORT).unwrap().map(|c| c.key), Some(5));
    }

    /// A process's exit is a signal like any other.
    #[test]
    fn a_process_exit_completes() {
        let port = CompletionPort::new(0).unwrap();
        let child = std::process::Command::new("cmd")
            .args(["/c", "exit", "0"])
            .spawn()
            .unwrap();
        let packet = WaitPacket::new().unwrap();
        packet.associate(&port, child.as_handle(), tagged(11)).unwrap();
        let got = port.dequeue(Some(Duration::from_secs(10))).unwrap();
        assert_eq!(got.map(|c| c.key), Some(11));
    }

    /// Waits and I/O share one port: a posted completion and a signal both
    /// arrive through the same dequeue.
    #[test]
    fn signals_and_posts_share_a_port() {
        let port = CompletionPort::new(0).unwrap();
        let event = Event::new();
        let packet = WaitPacket::new().unwrap();
        packet.associate(&port, event.0.as_handle(), tagged(1)).unwrap();
        port.post(tagged(2)).unwrap();
        event.set();
        let mut got = Vec::new();
        while got.len() < 2 && port.dequeue_many(&mut got, 4, SHORT).unwrap() > 0 {}
        let mut keys: Vec<_> = got.iter().map(|c| c.key).collect();
        keys.sort();
        assert_eq!(keys, [1, 2]);
    }
}
