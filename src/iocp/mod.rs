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

/// Refuses the key the overlapped core's own operations arrive under:
/// a completion forged with it would be dereferenced by
/// [`CompletionPort::dispatch`] as an operation of the crate's.
fn not_reserved(key: usize) -> Result<()> {
    if key == crate::common::overlapped::port_key() {
        Err(Error::new(
            windows::Win32::Foundation::E_INVALIDARG,
            "this completion key is reserved for the crate's own operations",
        ))
    } else {
        Ok(())
    }
}

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
///
/// Besides handles the caller associates, it can carry this crate's own
/// overlapped operations (`http::HttpServer::bind_to_port`): those arrive
/// under a key of the crate's, and the loop hands every completion to
/// [`dispatch`](Self::dispatch), which finishes the ones that are the
/// crate's and returns `false` for the rest.
pub struct CompletionPort {
    // Shared with the crate's registrations on this port, which must not
    // outlive it.
    handle: std::sync::Arc<OwnedHandle>,
}

impl CompletionPort {
    /// A new port. `concurrency` is how many threads the port lets run at
    /// once; 0 means one per processor.
    pub fn new(concurrency: u32) -> Result<CompletionPort> {
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, concurrency)? };
        Ok(CompletionPort {
            handle: std::sync::Arc::new(unsafe { OwnedHandle::from_raw_handle(port.0) }),
        })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    /// This port, for registering the crate's own handles on it.
    #[cfg(feature = "http")]
    pub(crate) fn port_ref(&self) -> crate::common::overlapped::PortRef {
        crate::common::overlapped::PortRef::new(self.handle.clone())
    }

    /// Finishes `completion` if it is one of this crate's own operations
    /// queued here -- runs its completion and wakes its future -- and
    /// returns `true`; returns `false`, touching nothing, for any other.
    /// Every completion dequeued from a port the crate's operations use must
    /// come through here, or those operations never finish.
    pub fn dispatch(&self, completion: &Completion) -> bool {
        // SAFETY: only the overlapped core associates or posts under its
        // key -- `associate`, `post` and the wait and job registrations here
        // refuse it -- so a completion carrying it is the core's.
        unsafe {
            crate::common::overlapped::dispatch(completion.key, completion.overlapped as *mut OVERLAPPED)
        }
    }

    /// Associates `handle` -- opened for overlapped I/O -- so its
    /// completions arrive here with `key`. A handle can be associated with
    /// one port, for good.
    pub fn associate(&self, handle: BorrowedHandle<'_>, key: usize) -> Result<()> {
        not_reserved(key)?;
        unsafe { CreateIoCompletionPort(HANDLE(handle.as_raw_handle()), Some(self.raw()), key, 0)? };
        Ok(())
    }

    /// Queues `completion` as if an operation had finished. Its `status` is
    /// not carried: a posted completion always reads as 0.
    pub fn post(&self, completion: Completion) -> Result<()> {
        not_reserved(completion.key)?;
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
        not_reserved(completion.key)?;
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

// ---------------------------------------------------------------------------
// Job object notifications
// ---------------------------------------------------------------------------

/// A job object whose events -- a process joining or leaving, limits
/// reached, the last process gone -- arrive at a [`CompletionPort`] as
/// completions; [`JobMessage::from_completion`] reads them.
///
/// Windows does not promise every message (only limit notifications are
/// guaranteed), so treat them as prompts, not as an audit log.
pub struct Job {
    handle: OwnedHandle,
}

impl Job {
    /// A new, unnamed job.
    pub fn new() -> Result<Job> {
        use windows::Win32::System::JobObjects::CreateJobObjectW;
        let job = unsafe { CreateJobObjectW(None, windows_core::PCWSTR::null())? };
        Ok(Job {
            handle: unsafe { OwnedHandle::from_raw_handle(job.0) },
        })
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    /// Sends this job's messages to `port`, tagged with `key`. A job reports
    /// to one port, and the association cannot be moved.
    pub fn notify(&self, port: &CompletionPort, key: usize) -> Result<()> {
        not_reserved(key)?;
        use windows::Win32::System::JobObjects::{
            JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JobObjectAssociateCompletionPortInformation,
            SetInformationJobObject,
        };
        let info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: key as *mut core::ffi::c_void,
            CompletionPort: port.raw(),
        };
        unsafe {
            SetInformationJobObject(
                self.raw(),
                JobObjectAssociateCompletionPortInformation,
                (&info as *const JOBOBJECT_ASSOCIATE_COMPLETION_PORT).cast(),
                size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
            )
        }
    }

    /// Ends every process in the job when the job's last handle closes --
    /// so dropping this `Job` takes its processes with it.
    pub fn kill_on_close(&self) -> Result<()> {
        use windows::Win32::System::JobObjects::{
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                self.raw(),
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
    }

    /// Adds a process. Processes it creates join too, unless created to
    /// break away.
    pub fn assign(&self, process: BorrowedHandle<'_>) -> Result<()> {
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;
        unsafe { AssignProcessToJobObject(self.raw(), HANDLE(process.as_raw_handle())) }
    }

    /// Ends every process in the job with `exit_code`.
    pub fn terminate(&self, exit_code: u32) -> Result<()> {
        use windows::Win32::System::JobObjects::TerminateJobObject;
        unsafe { TerminateJobObject(self.raw(), exit_code) }
    }
}

impl AsHandle for Job {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }
}

/// A job's message, read from the completion that carried it: the message
/// ID comes as the byte count and the process ID, where there is one, as
/// the `OVERLAPPED` value (`JOB_OBJECT_MSG_*` in winnt.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobMessage {
    /// A process joined the job (`JOB_OBJECT_MSG_NEW_PROCESS`).
    NewProcess(u32),
    /// A process left by exiting (`JOB_OBJECT_MSG_EXIT_PROCESS`).
    ExitProcess(u32),
    /// A process left by an unhandled exception or the like.
    AbnormalExitProcess(u32),
    /// The job has no processes left.
    ActiveProcessZero,
    ActiveProcessLimit,
    EndOfJobTime,
    EndOfProcessTime(u32),
    ProcessMemoryLimit(u32),
    JobMemoryLimit(u32),
    NotificationLimit,
    /// A message this crate does not name.
    Other { id: u32, process_id: u32 },
}

impl JobMessage {
    pub fn from_completion(completion: &Completion) -> JobMessage {
        let pid = completion.overlapped as u32;
        match completion.bytes {
            1 => Self::EndOfJobTime,
            2 => Self::EndOfProcessTime(pid),
            3 => Self::ActiveProcessLimit,
            4 => Self::ActiveProcessZero,
            6 => Self::NewProcess(pid),
            7 => Self::ExitProcess(pid),
            8 => Self::AbnormalExitProcess(pid),
            9 => Self::ProcessMemoryLimit(pid),
            10 => Self::JobMemoryLimit(pid),
            11 => Self::NotificationLimit,
            id => Self::Other { id, process_id: pid },
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod replace_probe_tests;
