//! Where connections come from -- one of the three transports -- and
//! where an armed accept delivers.

use super::*;

/// How a listener issues one accept.
///
/// A trait rather than an enum because each transport is now its own COM
/// type: the shared accept chain only ever needs "arm one accept", and
/// keeping that the whole contract is what lets the three listeners share
/// every other line of this module.
///
/// No transport does kernel work when it is bound. The named-pipe source
/// holds only the name until its first `arm` (which creates the first
/// instance, and from the first connection on keeps one in reserve); the socket
/// sources hold the address and bind/listen *lazily* on the first `arm`, then
/// reuse the one listen socket. Deferring that bind is what makes handler
/// gating complete: with no handler the chain never arms, so no listen socket
/// exists and a client cannot connect into a backlog. The cost is that a bind
/// error — a stale AF_UNIX socket file, `WSAEAFNOSUPPORT` where the Hyper-V
/// provider is absent — surfaces when the chain first starts rather than from
/// the bind call.
///
/// Held behind an `Arc` in `ListenerState::source` and cloned out before use,
/// so `try_arm` can drop that lock before arming.
/// Where a listener's connections come from: one of the three transports.
pub(crate) enum AcceptSource {
    /// The pipe's name and its instances. Dropping it -- on `Close`, or
    /// when a rebind replaces it -- closes the instance held in reserve,
    /// which is what gives the name up.
    Pipe(Arc<PipeServer>),
    Unix(UnixSource),
    HyperV(HyperVSource),
}

impl AcceptSource {
    /// Issues one accept through the transport's callback-based primitive
    /// (`pipe::arm_accept` / `*StreamListener::arm_accept`), each built
    /// directly on `tpio::submit` — no `Future`, no executor.
    ///
    /// `Ok(None)` means nothing could be armed yet -- a named pipe at its
    /// instance limit -- and `on_result` will be given
    /// [`crate::comio::pipe::SLOT_FREED`] once something can be.
    pub(super) fn arm(&self, on_result: AcceptSink) -> Result<Option<OpHandle>> {
        match self {
            AcceptSource::Pipe(server) => crate::comio::pipe::arm_accept(server, on_result),
            AcceptSource::Unix(source) => {
                let mut guard = source.listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(crate::comio::UnixStreamListener::bind(
                        &source.path,
                        source.security.as_ref(),
                    )?);
                }
                guard.as_ref().unwrap().arm_accept(on_result).map(Some)
            }
            AcceptSource::HyperV(source) => {
                let mut guard = source.listener.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(crate::comio::HvStreamListener::bind(source.addr)?);
                }
                guard.as_ref().unwrap().arm_accept(on_result).map(Some)
            }
        }
    }
}

pub(crate) struct UnixSource {
    pub(super) path: String,
    pub(super) security: Option<SecurityDescriptor>,
    /// `None` until the first `arm` binds the persistent listen socket.
    pub(super) listener: Mutex<Option<crate::comio::UnixStreamListener>>,
}

pub(crate) struct HyperVSource {
    pub(super) addr: HvSocketAddr,
    pub(super) listener: Mutex<Option<crate::comio::HvStreamListener>>,
}

/// Where an armed accept delivers: the listener's chain, which fires the
/// event and re-arms (see [`drive`]). One per armed accept; delivered
/// exactly once, by the completion or by the slot waiter that parked it.
pub(crate) struct AcceptSink {
    pub(super) state: Arc<ListenerState>,
}
