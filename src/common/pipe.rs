//! The server half of a named pipe's name: creating instances so that the
//! name is ours from the first one and never left unowned between accepts.
//! Shared by `tp` and `comio`, whose listeners differ only in how they wait.
//!
//! Three defaults, none of them optional:
//!
//! - **The first instance claims the name** (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
//!   If any other server already holds it, binding fails instead of quietly
//!   adding an instance to someone else's pipe -- the "pipe squatting" attack,
//!   where an earlier process owns the name and receives our clients.
//! - **The name is never left unowned.** Once a client connects, the next
//!   instance is created before the connected one is handed on, so there is
//!   always one listening instance until the listener is dropped. Without it,
//!   a client that connects and immediately disconnects would leave a gap in
//!   which another process could create the name afresh.
//! - **Remote clients are refused** (`PIPE_REJECT_REMOTE_CLIENTS`). These
//!   are local transports; a pipe is otherwise reachable over SMB by anyone
//!   with credentials on this machine.

use crate::common::security::SecurityDescriptor;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_core::{Error, HSTRING, Result};

/// A listening instance held in reserve. A raw `HANDLE` is not `Send`; this
/// one is owned by exactly one `PipeServer` at a time.
struct Spare(HANDLE);
unsafe impl Send for Spare {}

/// Creates the instances of one pipe name.
pub(crate) struct PipeServer {
    name: HSTRING,
    security: Option<SecurityDescriptor>,
    /// Set once an instance has been created: from then on the name is ours
    /// and later instances must not ask to be first.
    claimed: AtomicBool,
    spare: Mutex<Option<Spare>>,
}

impl PipeServer {
    pub(crate) fn new(name: &str, security: Option<SecurityDescriptor>) -> Self {
        Self {
            name: HSTRING::from(name),
            security,
            claimed: AtomicBool::new(false),
            spare: Mutex::new(None),
        }
    }

    pub(crate) fn name(&self) -> &HSTRING {
        &self.name
    }

    /// A listening instance to connect: the one held in reserve if there is
    /// one, otherwise a new one. The caller owns the handle.
    ///
    /// A reserved instance may already have a client: one can connect to an
    /// instance before `ConnectNamedPipe` is called on it, which then reports
    /// `ERROR_PIPE_CONNECTED`. Every accept path here treats that as success.
    pub(crate) fn instance(&self) -> Result<HANDLE> {
        if let Some(Spare(h)) = self.spare.lock().unwrap().take() {
            return Ok(h);
        }
        self.create()
    }

    /// Called once a client has connected to an instance, while that instance
    /// still holds the name: creates the next listening instance now, so the
    /// name stays ours whatever the connected one's owner does with it.
    ///
    /// Best effort. If it fails the next accept creates an instance itself,
    /// as it would have anyway.
    pub(crate) fn replenish(&self) {
        let mut spare = self.spare.lock().unwrap();
        if spare.is_none()
            && let Ok(h) = self.create()
        {
            *spare = Some(Spare(h));
        }
    }

    fn create(&self) -> Result<HANDLE> {
        let first = !self.claimed.load(Ordering::Acquire);
        let mut open_mode = PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE.0;
        }
        let attributes = self.security.as_ref().map(SecurityDescriptor::attributes);
        let handle = unsafe {
            CreateNamedPipeW(
                &self.name,
                FILE_FLAGS_AND_ATTRIBUTES(open_mode),
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                0,
                attributes.as_ref().map(|a| a as *const _),
            )
        };
        // A bare `HANDLE`, not a `Result`: failure is `INVALID_HANDLE_VALUE`
        // plus `GetLastError()`.
        if handle.is_invalid() {
            let e = Error::from_thread();
            if first && e.code() == ERROR_ACCESS_DENIED.to_hresult() {
                return Err(Error::new(
                    e.code(),
                    "the pipe name is already held by another server",
                ));
            }
            return Err(e);
        }
        self.claimed.store(true, Ordering::Release);
        Ok(handle)
    }
}

/// Replenishes `server` when dropped, whichever way the accept holding it
/// ends: connected, failed, or its future dropped mid-wait.
///
/// A future-based accept creates this right after registering its instance,
/// so it is declared *after* that registration and drops *before* it: the
/// next instance is created while this one still holds the name. An explicit
/// call at the end would miss the dropped-future case, which closes the
/// instance on its way out.
pub(crate) struct ReplenishOnExit<'a>(pub(crate) &'a PipeServer);

impl Drop for ReplenishOnExit<'_> {
    fn drop(&mut self) {
        self.0.replenish();
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        if let Some(Spare(h)) = self.spare.get_mut().unwrap().take() {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}
