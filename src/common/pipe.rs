//! The server half of a named pipe's name: creating instances so that the
//! name is ours from the first one and never joined to someone else's pipe.
//! Shared by `tp` and `comio`, whose listeners differ only in how they wait.
//!
//! Three defaults:
//!
//! - **Whenever we hold no instance, the next one claims the name**
//!   (`FILE_FLAG_FIRST_PIPE_INSTANCE`). If any other server holds it, the
//!   create fails instead of quietly adding an instance to someone else's
//!   pipe -- the "pipe squatting" attack, where an earlier process owns the
//!   name and receives our clients.
//! - **The name is held between accepts.** Once a client connects, the next
//!   instance is created before the connected one is handed on, so with the
//!   default instance limit there is always one listening instance until the
//!   listener is dropped. A lower [`NamedPipeOptions::max_instances`] can use
//!   that reserve up; the name is then free between the last disconnect and
//!   the next accept, and the rule above is what still keeps the next accept
//!   from joining a squatter.
//! - **Remote clients are refused** (`PIPE_REJECT_REMOTE_CLIENTS`) unless
//!   asked for, and asking requires a descriptor. These are local transports
//!   by default; a pipe is otherwise reachable over SMB by anyone the
//!   descriptor admits.
//!
//! With no descriptor given, instances get
//! [`security::default_pipe_descriptor`](crate::common::security), not the
//! system's default, which also admits Everyone and the anonymous account
//! for reading.

use crate::common::overlapped::IoHandle;
use crate::common::security::{self, SecurityDescriptor};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, E_INVALIDARG, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_SEM_TIMEOUT, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, GetNamedPipeInfo, NAMED_PIPE_MODE, PIPE_ACCEPT_REMOTE_CLIENTS,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, SetNamedPipeHandleState, WaitNamedPipeW,
};
use windows_core::{Error, HSTRING, Result};

/// How a named-pipe listener creates its instances.
///
/// The defaults are the ones every listener had before this type existed:
/// byte mode, local clients only, no instance limit, 4 KiB buffers each way,
/// and the descriptor from `default_pipe_descriptor`.
///
/// `PIPE_NOWAIT` is deliberately not offered. Microsoft keeps it for LAN
/// Manager 2.0 compatibility and points asynchronous callers at overlapped
/// I/O, which is what every instance here already uses.
#[derive(Clone, Debug)]
pub struct NamedPipeOptions {
    pub(crate) security: Option<SecurityDescriptor>,
    pub(crate) message_mode: bool,
    pub(crate) accept_remote_clients: bool,
    pub(crate) max_instances: u32,
    pub(crate) in_buffer: u32,
    pub(crate) out_buffer: u32,
}

impl Default for NamedPipeOptions {
    fn default() -> Self {
        Self {
            security: None,
            message_mode: false,
            accept_remote_clients: false,
            max_instances: PIPE_UNLIMITED_INSTANCES,
            in_buffer: 4096,
            out_buffer: 4096,
        }
    }
}

impl NamedPipeOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Who may connect. It must grant this process's own account
    /// `FILE_CREATE_PIPE_INSTANCE` (`GA` does): every instance after the
    /// first is checked against it.
    pub fn security(mut self, security: &SecurityDescriptor) -> Self {
        self.security = Some(security.clone());
        self
    }

    /// Message mode (`PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE`): each write
    /// is one message, and a read never returns parts of two. A connector
    /// notices and switches its own end to message reads.
    pub fn message_mode(mut self, on: bool) -> Self {
        self.message_mode = on;
        self
    }

    /// Admits clients on other machines (`PIPE_ACCEPT_REMOTE_CLIENTS`).
    /// Binding fails with `E_INVALIDARG` unless [`security`](Self::security)
    /// is set as well: opening a pipe to the network is a decision about
    /// *who*, and the default descriptor was written for local use.
    pub fn accept_remote_clients(mut self, on: bool) -> Self {
        self.accept_remote_clients = on;
        self
    }

    /// The most instances the pipe may have at once, 1 to 254, or 255
    /// (`PIPE_UNLIMITED_INSTANCES`, the default) for no limit.
    ///
    /// Every connected client holds one, and so does the listening instance
    /// the listener keeps in reserve. At the limit an accept waits for an
    /// instance to close, and a client's connect waits too (its
    /// `ERROR_PIPE_BUSY`). When every instance has been connected and then
    /// closed, none exists until the next accept creates one, and a client
    /// connecting in between gets `ERROR_FILE_NOT_FOUND`. See the module docs
    /// for what that gap does to the name.
    pub fn max_instances(mut self, n: u32) -> Self {
        self.max_instances = n;
        self
    }

    /// The sizes the system reserves for each instance's buffers, inbound
    /// (client to server) and outbound. Advisory: the system may round them.
    /// In message mode a message larger than the outbound buffer still goes
    /// through; the write just waits for the reader.
    pub fn buffer_sizes(mut self, inbound: u32, outbound: u32) -> Self {
        self.in_buffer = inbound;
        self.out_buffer = outbound;
        self
    }

    /// Whether these options describe a pipe that can be created.
    pub(crate) fn validate(&self) -> Result<()> {
        if !(1..=PIPE_UNLIMITED_INSTANCES).contains(&self.max_instances) {
            return Err(Error::new(
                E_INVALIDARG,
                "max_instances must be 1 to 254, or 255 for no limit",
            ));
        }
        if self.accept_remote_clients && self.security.is_none() {
            return Err(Error::new(
                E_INVALIDARG,
                "accepting remote clients needs an explicit security descriptor",
            ));
        }
        Ok(())
    }

    fn pipe_mode(&self) -> NAMED_PIPE_MODE {
        let kind = if self.message_mode {
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE
        } else {
            PIPE_TYPE_BYTE
        };
        let remote = if self.accept_remote_clients {
            PIPE_ACCEPT_REMOTE_CLIENTS
        } else {
            PIPE_REJECT_REMOTE_CLIENTS
        };
        kind | PIPE_WAIT | remote
    }
}

/// Called when an instance of ours closes (`true`) or the server is dropped
/// (`false`). Never called under a lock.
/// Who is told when an instance closes at the limit: `true` for an instance
/// freed, `false` for the server going away.
pub(crate) enum SlotWaiter {
    /// An `accept` future, waiting on the channel.
    Signal(crate::common::oneshot::Sender<bool>),
    /// comio's executor-free accept chain, parked until it can arm.
    #[cfg(feature = "comio")]
    Accept(crate::comio::pipe::ParkedAccept),
}

impl SlotWaiter {
    fn wake(self, freed: bool) {
        match self {
            SlotWaiter::Signal(tx) => {
                let _ = tx.send(freed);
            }
            #[cfg(feature = "comio")]
            SlotWaiter::Accept(parked) => parked.wake(freed),
        }
    }
}

/// How many instances of the name this process holds, and who is waiting
/// for one to close.
#[derive(Default)]
struct Slots {
    live: usize,
    waiters: Vec<SlotWaiter>,
}

/// Counts one live instance; dropped after the instance's handle is closed.
///
/// Handed to the instance's I/O registration as `closes_with`, which the
/// registration drops after closing the handle -- so the count never says "none"
/// while a handle of ours is still open, which would make the next create
/// ask to be first and fail against our own pipe. It can say "one" for a
/// moment after the last handle closes; a create in that moment does not ask
/// to be first, which is the narrow gap the module docs describe.
pub(crate) struct InstanceToken(Arc<Mutex<Slots>>);

impl Drop for InstanceToken {
    fn drop(&mut self) {
        let waiters = {
            let mut slots = self.0.lock().unwrap();
            slots.live -= 1;
            std::mem::take(&mut slots.waiters)
        };
        for w in waiters {
            w.wake(true);
        }
    }
}

/// How long to wait before trying again when the pipe is full of instances
/// none of which are ours to watch. See [`Slot::Retry`].
pub(crate) const RETRY_AFTER: Duration = Duration::from_millis(50);

/// What [`PipeServer::instance_or_wait`] has for an accept.
pub(crate) enum Slot {
    /// A listening instance.
    Ready(Instance),
    /// At the limit; the waiter runs when an instance of ours closes.
    Wait,
    /// At the limit, but with no instance of ours open, so nothing will call
    /// the waiter: the instances still counted are ones whose server end we
    /// closed while the client kept its end open. The kernel counts an
    /// instance until *both* ends close, and when a client closes is not
    /// something a server can observe -- so try again after [`RETRY_AFTER`].
    Retry,
}

/// A created instance: its handle, and the count it holds. Dropping it
/// closes the handle, then (field order) releases the count.
pub struct Instance {
    pub(crate) handle: HANDLE,
    token: InstanceToken,
}

// The handle is owned by exactly one `Instance` at a time.
unsafe impl Send for Instance {}
unsafe impl Sync for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

impl From<Instance> for IoHandle {
    fn from(instance: Instance) -> IoHandle {
        IoHandle::PipeInstance(instance)
    }
}

/// Creates the instances of one pipe name.
pub(crate) struct PipeServer {
    name: HSTRING,
    options: NamedPipeOptions,
    slots: Arc<Mutex<Slots>>,
    spare: Mutex<Option<Instance>>,
}

impl PipeServer {
    /// `options` must already have passed `validate`.
    pub(crate) fn new(name: &str, options: NamedPipeOptions) -> Self {
        Self {
            name: HSTRING::from(name),
            options,
            slots: Arc::default(),
            spare: Mutex::new(None),
        }
    }

    pub(crate) fn name(&self) -> &HSTRING {
        &self.name
    }

    pub(crate) fn message_mode(&self) -> bool {
        self.options.message_mode
    }

    /// A listening instance to connect: the one held in reserve if there is
    /// one, otherwise a new one. At the instance limit, see [`Slot`]: `waiter`
    /// is kept only for [`Slot::Wait`].
    ///
    /// A reserved instance may already have a client: one can connect to an
    /// instance before `ConnectNamedPipe` is called on it, which then reports
    /// `ERROR_PIPE_CONNECTED`. Every accept path here treats that as success.
    pub(crate) fn instance_or_wait(&self, waiter: SlotWaiter) -> Result<Slot> {
        if let Some(spare) = self.spare.lock().unwrap().take() {
            return Ok(Slot::Ready(spare));
        }
        // Create and register under one lock, so an instance closing between
        // a failed create and the registration still finds the waiter.
        let mut slots = self.slots.lock().unwrap();
        match self.create(&mut slots) {
            Ok(instance) => Ok(Slot::Ready(instance)),
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                if slots.live > 0 {
                    slots.waiters.push(waiter);
                    Ok(Slot::Wait)
                } else {
                    Ok(Slot::Retry)
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Called once a client has connected to an instance, while that instance
    /// still holds the name: creates the next listening instance now, so the
    /// name stays ours whatever the connected one's owner does with it.
    ///
    /// Best effort. At the instance limit, or if the create fails, the next
    /// accept creates an instance itself, as it would have anyway.
    pub(crate) fn replenish(&self) {
        let mut spare = self.spare.lock().unwrap();
        if spare.is_none()
            && let Ok(instance) = self.create(&mut self.slots.lock().unwrap())
        {
            *spare = Some(instance);
        }
    }

    fn create(&self, slots: &mut Slots) -> Result<Instance> {
        let first = slots.live == 0;
        let mut open_mode = PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE.0;
        }
        let security = match &self.options.security {
            Some(sd) => sd.clone(),
            None => security::default_pipe_descriptor()?,
        };
        let attributes = security.attributes();
        let handle = unsafe {
            CreateNamedPipeW(
                &self.name,
                FILE_FLAGS_AND_ATTRIBUTES(open_mode),
                self.options.pipe_mode(),
                self.options.max_instances,
                self.options.out_buffer,
                self.options.in_buffer,
                0,
                Some(&attributes),
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
        slots.live += 1;
        Ok(Instance {
            handle,
            token: InstanceToken(self.slots.clone()),
        })
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
        drop(self.spare.get_mut().unwrap().take());
        // Anyone still waiting for a slot is waiting on a listener that is
        // gone; tell them so rather than leave them parked.
        let waiters = std::mem::take(&mut self.slots.lock().unwrap().waiters);
        for w in waiters {
            w.wake(false);
        }
    }
}

/// How long a connect waits, in total, while every instance is busy, unless
/// the caller says otherwise.
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The client end of a pipe, opened and matched to the pipe's mode.
pub(crate) struct ClientEnd {
    pub(crate) handle: HANDLE,
    /// The pipe is message-mode, and this end now reads messages.
    pub(crate) message_mode: bool,
}

/// One attempt to open the client end of `name`, with no waiting.
///
/// Identification level, not the default impersonation: the server may learn
/// who we are, but cannot act as us. Without `SECURITY_SQOS_PRESENT` a client
/// grants full impersonation to whatever answers on the name.
///
/// A client end always opens in byte read mode. On a message-mode pipe it is
/// switched to message reads here, so a connector need not know in advance
/// which kind of pipe it is reaching; `ClientEnd::message_mode` says which.
pub(crate) fn open_client(name: &HSTRING) -> Result<ClientEnd> {
    let handle = unsafe {
        CreateFileW(
            name,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )?
    };
    let matched = (|| {
        let mut flags = NAMED_PIPE_MODE(0);
        unsafe { GetNamedPipeInfo(handle, Some(&mut flags), None, None, None)? };
        let message_mode = flags.0 & PIPE_TYPE_MESSAGE.0 != 0;
        if message_mode {
            unsafe { SetNamedPipeHandleState(handle, Some(&PIPE_READMODE_MESSAGE), None, None)? };
        }
        Ok(message_mode)
    })();
    match matched {
        Ok(message_mode) => Ok(ClientEnd {
            handle,
            message_mode,
        }),
        Err(e) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(e)
        }
    }
}

/// [`open_client`], waiting up to `timeout` in total while every instance is
/// busy. A zero `timeout` does not wait.
///
/// Blocks the calling thread for as long as it waits: `WaitNamedPipeW` has no
/// overlapped form. That is also why the wait is always bounded, and why the
/// two special values the API gives meaning to are never passed --
/// `NMPWAIT_WAIT_FOREVER` would pin the thread for good, and
/// `NMPWAIT_USE_DEFAULT_WAIT` (0) would wait the *server's* choice instead of
/// ours.
///
/// Running out of time is `ERROR_PIPE_BUSY`, the condition that was waited
/// on, rather than the `ERROR_SEM_TIMEOUT` the wait itself reports.
pub(crate) fn open_client_waiting(name: &HSTRING, timeout: Duration) -> Result<ClientEnd> {
    let deadline = Instant::now() + timeout;
    let busy = || Error::new(ERROR_PIPE_BUSY.to_hresult(), "every instance stayed busy");
    loop {
        match open_client(name) {
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {}
            other => return other,
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(busy());
        }
        // 1 ..= u32::MAX - 1: never 0 (server's default) or u32::MAX (forever).
        let ms = left.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32;
        match unsafe { WaitNamedPipeW(name, ms) }.ok() {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_SEM_TIMEOUT.to_hresult() => return Err(busy()),
            Err(e) => return Err(e),
        }
        // An instance freed up -- but another client may take it first, in
        // which case the open is busy again and the loop waits out the rest.
    }
}
