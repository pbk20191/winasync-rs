//! Connection points: the listeners' event, delivered to a script.
//!
//! [`super::dispatch`] gave a script a blocking `Accept`, because that was
//! all it could use. This is the other half of the answer to the same
//! problem: with a type library declaring a `[default, source]` interface
//! ([`super::typelib`]), a host can hand the listener a sink and be *called*
//! when a connection arrives, instead of waiting for one.
//!
//! That is what `ConnectionReceived` always was for a compiled caller. It
//! was unreachable from a script only because a `TypedEventHandler` cannot
//! be constructed there and nothing advertised where to attach. Neither is
//! true any more, so `Accept` and this are now two shapes of one event
//! rather than one real path and one workaround: a script that wants to do
//! something else while waiting advises a sink, and one that just wants the
//! next connection calls `Accept`.
//!
//! # What a host does
//!
//! `QueryInterface` for `IConnectionPointContainer`, `FindConnectionPoint`
//! with the source IID, `Advise` with its sink. Finding the source IID is
//! what [`windows::Win32::System::Ole::IProvideClassInfo2`] is for — .NET's
//! COM event support, which is what PowerShell's `Register-ObjectEvent`
//! uses, asks for `GUIDKIND_DEFAULT_SOURCE_DISP_IID` rather than reading the
//! library. Both are implemented, and they agree.
//!
//! # Apartments
//!
//! A sink is advised on the host's thread and called from the thread-pool
//! thread an accept completes on. Those are not the same apartment, and a
//! raw interface pointer may not cross. Each sink is therefore kept as an
//! [`AgileReference`], which parks it in the global interface table and
//! marshals it back on the firing thread — the same mechanism `asyncop`
//! uses to carry a buffer to its completion. The dispinterface is
//! registered with oleaut32's dispatch proxy, which is what makes that
//! marshalling possible at all.
//!
//! # Which hosts this actually reaches
//!
//! Verified end to end, in `tests/activation.rs`: Windows Script Host.
//! `WScript.ConnectObject` performs the whole discovery — class info, the
//! coclass's default source interface, `FindConnectionPoint`, `Advise` —
//! and a `Sub L_ConnectionReceived(stream)` is called with a stream it then
//! reads from, across the apartment boundary. VBA's `WithEvents` and an ATL
//! sink take that same path.
//!
//! PowerShell is the exception, and not for a reason this code can fix.
//! `Register-ObjectEvent` resolves an event by name through
//! `Type.GetEvent`, which a `System.__ComObject` has nothing to answer with
//! unless an interop assembly was imported from the library first — COM
//! discovery is not consulted at all. Going under it with
//! `ComEventsHelper.Combine` and an explicit IID and DISPID does find the
//! connection point and advise successfully, but the sink it builds then
//! calls a PowerShell script block on the thread the event is raised on,
//! where no runspace exists. Both are host limitations above COM; the
//! blocking `Accept` in [`super::dispatch`] is what PowerShell should use,
//! which is one reason it is still there.
//!
//! # Enumerators
//!
//! `EnumConnectionPoints` and `EnumConnections` return `E_NOTIMPL`. They
//! exist for design-time browsers that want to list what an object offers;
//! every host that actually wires up events — .NET, VBA, VBScript — goes
//! through `FindConnectionPoint` with an IID it already knows, which is the
//! path above. Implementing the enumerators would mean two more COM objects
//! serving callers this crate has none of.

use super::typelib::DISPID_CONNECTION_RECEIVED;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::Win32::Foundation::E_NOTIMPL;
use windows::Win32::System::Ole::{
    CONNECT_E_ADVISELIMIT, CONNECT_E_CANNOTCONNECT, CONNECT_E_NOCONNECTION,
};
use windows::Win32::System::Com::{
    DISPATCH_METHOD, DISPPARAMS, IConnectionPoint, IConnectionPointContainer,
    IConnectionPoint_Impl, IDispatch, IEnumConnections,
};
use windows_core::{AgileReference, GUID, IInspectable, IUnknown, Interface, Ref, Result, implement};

/// The sinks advised on one listener.
///
/// Lives in `ListenerCore` rather than in the connection point, because
/// `FindConnectionPoint` hands out a fresh connection-point object on every
/// call and all of them must see the same set.
pub(crate) struct SinkSet {
    sinks: Mutex<Vec<(u32, AgileReference<IDispatch>)>>,
    /// Cookies are never reused, so a stale `Unadvise` cannot detach
    /// somebody else's sink. Starts at 1: zero is not a valid cookie.
    next_cookie: AtomicU32,
}

impl SinkSet {
    pub(crate) fn new() -> Self {
        Self {
            sinks: Mutex::new(Vec::new()),
            next_cookie: AtomicU32::new(1),
        }
    }

    /// Keeps the process MTA alive for as long as a sink could fire.
    ///
    /// The accept chain runs on thread-pool threads, which nobody has
    /// called `CoInitializeEx` on. A call from such a thread is served by
    /// the *implicit* MTA — but only while an MTA exists, and in a host
    /// whose own threads are all STA (`cscript`, and any other single-
    /// threaded script host) none does. Without this the first `Invoke` on
    /// a sink fails with `CO_E_NOTINITIALIZED` and the event is simply lost.
    ///
    /// `CoIncrementMTAUsage` exists for exactly this: it keeps MTA support
    /// up without parking a thread in it. The cookie is deliberately never
    /// released — the MTA must outlive every sink, and a listener can be
    /// advised again at any time, so there is no point at which giving it
    /// back would be safe.
    fn ensure_mta() {
        static MTA: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        MTA.get_or_init(|| {
            let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() };
        });
    }

    /// Adds a sink, which must be callable through `IDispatch`.
    ///
    /// The source is a dispinterface, so that is the only shape a sink can
    /// have — a host that passes something else gets
    /// `CONNECT_E_CANNOTCONNECT`, which is the contract's way of saying the
    /// object is not the right kind.
    pub(crate) fn advise(&self, sink: Ref<IUnknown>) -> Result<u32> {
        let sink = sink.ok()?;
        let dispatch: IDispatch = sink.cast().map_err(|_| {
windows_core::Error::from_hresult(CONNECT_E_CANNOTCONNECT)
        })?;
        Self::ensure_mta();
        let agile = AgileReference::new(&dispatch)?;
        let cookie = self.next_cookie.fetch_add(1, Ordering::Relaxed);
        if cookie == 0 {
            return Err(windows_core::Error::from_hresult(CONNECT_E_ADVISELIMIT));
        }
        self.sinks.lock().unwrap().push((cookie, agile));
        Ok(cookie)
    }

    pub(crate) fn unadvise(&self, cookie: u32) -> Result<()> {
        let mut sinks = self.sinks.lock().unwrap();
        match sinks.iter().position(|(c, _)| *c == cookie) {
            Some(i) => {
                sinks.remove(i);
                Ok(())
            }
            None => Err(windows_core::Error::from_hresult(CONNECT_E_NOCONNECTION)),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sinks.lock().unwrap().is_empty()
    }

    /// Raises `ConnectionReceived(stream)` on every advised sink.
    ///
    /// Runs on the thread-pool thread that completed the accept. Nothing
    /// here may propagate a failure: this is the accept chain's callback,
    /// and one host's broken sink must not stop the listener serving the
    /// others. A sink that reports its client is gone is dropped instead,
    /// which is what keeps a crashed host from accumulating forever.
    pub(crate) fn fire(&self, stream: &IInspectable) {
        let Ok(arg) = super::dispatch::object(stream) else {
            return;
        };
        let targets: Vec<(u32, AgileReference<IDispatch>)> = {
            let sinks = self.sinks.lock().unwrap();
            sinks
                .iter()
                .map(|(c, a)| (*c, a.clone()))
                .collect()
        };

        let mut dead = Vec::new();
        for (cookie, agile) in targets {
            let Ok(sink) = agile.resolve() else {
                dead.push(cookie);
                continue;
            };
            // One argument, so no reversal to think about — but the
            // VARIANT is borrowed by the callee, not given away, which is
            // why the clone lives until after the call.
            let mut args = [arg.clone()];
            let params = DISPPARAMS {
                rgvarg: args.as_mut_ptr(),
                rgdispidNamedArgs: core::ptr::null_mut(),
                cArgs: 1,
                cNamedArgs: 0,
            };
            let hr = unsafe {
                sink.Invoke(
                    DISPID_CONNECTION_RECEIVED,
                    &GUID::zeroed(),
                    0,
                    DISPATCH_METHOD,
                    &params,
                    None,
                    None,
                    None,
                )
            };
            if let Err(e) = hr
                && is_gone(e.code())
            {
                dead.push(cookie);
            }
        }

        if !dead.is_empty() {
            let mut sinks = self.sinks.lock().unwrap();
            sinks.retain(|(c, _)| !dead.contains(c));
        }
    }
}

/// Whether an `Invoke` failure means the sink's client no longer exists, as
/// opposed to the sink's own handler raising an error.
///
/// Only the first is a reason to drop it; a handler that threw is still a
/// live subscriber that wants the next event.
fn is_gone(code: windows_core::HRESULT) -> bool {
    use windows::Win32::Foundation::{CO_E_OBJNOTCONNECTED, RPC_E_DISCONNECTED};
    code == RPC_E_DISCONNECTED
        || code == CO_E_OBJNOTCONNECTED
        || code == windows::Win32::Foundation::RPC_E_SERVERFAULT
        || code.0 as u32 == 0x800706BA // RPC_S_SERVER_UNAVAILABLE
}

/// What a connection point needs from the listener behind it.
///
/// Implemented in `listener.rs`, where the accept chain lives. The
/// connection point holds this rather than the listener itself so it does
/// not have to be generic over the three transports.
pub(crate) trait AdviseTarget: Send + Sync {
    /// Adds a sink and starts the accept chain if this is the first one.
    fn advise(&self, sink: Ref<IUnknown>) -> Result<u32>;
    /// Removes a sink; the chain pauses when the last consumer goes.
    fn unadvise(&self, cookie: u32) -> Result<()>;
}

/// One connection point, handed out by `FindConnectionPoint`.
///
/// Holds the container alive: a host may release the listener and keep the
/// connection point, and `GetConnectionPointContainer` has to keep working.
/// There is no cycle, because the listener does not hold a connection point
/// — a fresh one is made per call.
#[implement(IConnectionPoint)]
pub(crate) struct ConnectionPoint {
    target: std::sync::Arc<dyn AdviseTarget>,
    container: IConnectionPointContainer,
    iid: GUID,
}

impl ConnectionPoint {
    /// Returns the interface rather than `Self`: a connection point only
    /// ever leaves this module through a vtable, and `#[implement]` types
    /// are not usable as plain values anyway.
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(
        target: std::sync::Arc<dyn AdviseTarget>,
        container: IConnectionPointContainer,
        iid: GUID,
    ) -> IConnectionPoint {
        windows_core::ComObject::new(ConnectionPoint {
            target,
            container,
            iid,
        })
        .into_interface()
    }
}

impl IConnectionPoint_Impl for ConnectionPoint_Impl {
    fn GetConnectionInterface(&self) -> Result<GUID> {
        Ok(self.iid)
    }

    fn GetConnectionPointContainer(&self) -> Result<IConnectionPointContainer> {
        Ok(self.container.clone())
    }

    fn Advise(&self, sink: Ref<IUnknown>) -> Result<u32> {
        self.target.advise(sink)
    }

    fn Unadvise(&self, cookie: u32) -> Result<()> {
        self.target.unadvise(cookie)
    }

    /// Not implemented; see the module docs.
    fn EnumConnections(&self) -> Result<IEnumConnections> {
        Err(windows_core::Error::from_hresult(E_NOTIMPL))
    }
}

/// Generates `IConnectionPointContainer` and the two `IProvideClassInfo`
/// interfaces for a listener.
///
/// The bodies are identical across the three transports — they all raise
/// the same event — so only the class's own CLSID differs.
macro_rules! impl_connection_points {
    ($impl_ty:ident, $clsid:expr) => {
        // Raw-pointer parameters come from the `windows`-generated trait,
        // which is not `unsafe fn`; `riid` is null-checked before use.
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        impl windows::Win32::System::Com::IConnectionPointContainer_Impl for $impl_ty {
            /// Not implemented; see `connpt`'s module docs.
            fn EnumConnectionPoints(
                &self,
            ) -> windows_core::Result<windows::Win32::System::Com::IEnumConnectionPoints> {
                Err(windows_core::Error::from_hresult(
                    windows::Win32::Foundation::E_NOTIMPL,
                ))
            }

            fn FindConnectionPoint(
                &self,
                riid: *const windows_core::GUID,
            ) -> windows_core::Result<windows::Win32::System::Com::IConnectionPoint> {
                if riid.is_null() {
                    return Err(windows_core::Error::from_hresult(
                        windows::Win32::Foundation::E_POINTER,
                    ));
                }
                let riid = unsafe { *riid };
                if riid != $crate::comio::typelib::IID_DLISTENER_EVENTS {
                    // The contract's "I do not raise that": a host uses it
                    // to probe for optional events.
                    return Err(windows_core::Error::from_hresult(
                        windows::Win32::System::Ole::CONNECT_E_NOCONNECTION,
                    ));
                }
                let container: windows::Win32::System::Com::IConnectionPointContainer =
                    windows_core::IUnknownImpl::to_object(self).to_interface();
                Ok($crate::comio::connpt::ConnectionPoint::new(
                    self.core.advise_target(),
                    container,
                    riid,
                ))
            }
        }

        impl windows::Win32::System::Ole::IProvideClassInfo_Impl for $impl_ty {
            fn GetClassInfo(&self) -> windows_core::Result<windows::Win32::System::Com::ITypeInfo> {
                $crate::comio::typelib::type_info(&$clsid)
            }
        }

        impl windows::Win32::System::Ole::IProvideClassInfo2_Impl for $impl_ty {
            /// The source interface, which is how .NET — and so PowerShell's
            /// `Register-ObjectEvent` — finds what to advise without
            /// reading the type library.
            fn GetGUID(&self, kind: u32) -> windows_core::Result<windows_core::GUID> {
                if kind
                    != windows::Win32::System::Ole::GUIDKIND_DEFAULT_SOURCE_DISP_IID.0 as u32
                {
                    return Err(windows_core::Error::from_hresult(
                        windows::Win32::Foundation::E_INVALIDARG,
                    ));
                }
                Ok($crate::comio::typelib::IID_DLISTENER_EVENTS)
            }
        }
    };
}

pub(crate) use impl_connection_points;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::NamedPipeConnectionListener;
    use crate::comio::typelib::IID_DLISTENER_EVENTS;
    use std::sync::mpsc;
    use windows::Win32::System::Com::{
        COINIT_MULTITHREADED, CoInitializeEx, DISPATCH_FLAGS, EXCEPINFO, ITypeInfo,
    };
    use windows::Win32::System::Ole::{GUIDKIND_DEFAULT_SOURCE_DISP_IID, IProvideClassInfo2};
    use windows_core::PCWSTR;

    /// A sink of the shape a host supplies: dispatch-only, no vtable, no
    /// knowledge of this crate's types.
    ///
    /// It answers `Invoke` for the one DISPID the source declares and
    /// forwards whatever object it was handed, which is what lets the test
    /// check the event carried a usable stream rather than merely fired.
    #[implement(IDispatch)]
    struct Sink(mpsc::Sender<IDispatch>);

    impl windows::Win32::System::Com::IDispatch_Impl for Sink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }
        fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> {
            Err(windows_core::Error::from_hresult(E_NOTIMPL))
        }
        fn GetIDsOfNames(
            &self,
            _: *const GUID,
            _: *const PCWSTR,
            _: u32,
            _: u32,
            _: *mut i32,
        ) -> Result<()> {
            Err(windows_core::Error::from_hresult(E_NOTIMPL))
        }
        fn Invoke(
            &self,
            dispid: i32,
            _: *const GUID,
            _: u32,
            _: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _: *mut windows::Win32::System::Variant::VARIANT,
            _: *mut EXCEPINFO,
            _: *mut u32,
        ) -> Result<()> {
            assert_eq!(dispid, DISPID_CONNECTION_RECEIVED);
            let p = unsafe { &*params };
            assert_eq!(p.cArgs, 1, "ConnectionReceived takes one argument");
            let stream = unsafe {
                let v = &*p.rgvarg;
                let inner = &v.Anonymous.Anonymous;
                assert_eq!(
                    inner.vt,
                    windows::Win32::System::Variant::VT_DISPATCH,
                    "the stream should arrive as an object"
                );
                inner.Anonymous.pdispVal.as_ref().expect("null stream").clone()
            };
            let _ = self.0.send(stream);
            Ok(())
        }
    }

    /// The path a host takes, end to end: find the connection point by the
    /// IID the class advertises, advise a sink, and be called when a client
    /// connects — with a stream the sink can then use.
    #[test]
    fn an_advised_sink_is_called_with_a_working_stream() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().unwrap();
        let name = format!(r"\\.\pipe\comio_connpt_{}", std::process::id());

        let listener = NamedPipeConnectionListener::new();

        // The source IID comes from the object itself, not from a constant
        // the test happens to know — this is the discovery step .NET makes.
        let provider: IProvideClassInfo2 = listener.cast().expect("QI IProvideClassInfo2");
        let source = unsafe { provider.GetGUID(GUIDKIND_DEFAULT_SOURCE_DISP_IID.0 as u32) }
            .expect("GetGUID");
        assert_eq!(source, IID_DLISTENER_EVENTS);

        let container: IConnectionPointContainer = listener.cast().expect("QI container");
        let point = unsafe { container.FindConnectionPoint(&source) }.expect("FindConnectionPoint");
        assert_eq!(unsafe { point.GetConnectionInterface() }.unwrap(), source);

        let (tx, rx) = mpsc::channel();
        let sink: IUnknown = windows_core::ComObject::new(Sink(tx)).into_interface();
        let cookie = unsafe { point.Advise(&sink) }.expect("Advise");
        assert_ne!(cookie, 0);

        listener
            .bind(&windows_core::HSTRING::from(&name))
            .expect("bind")
            .join()
            .expect("bind result");

        let client = crate::comio::NamedPipeClient::new();
        let connected = client
            .connect_async(&windows_core::HSTRING::from(&name))
            .join()
            .expect("connect");

        let stream = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the sink was never called");

        // Not just a pointer: the object the event carried is the stream,
        // and it answers the members the library declares for one.
        let wrote = crate::comio::dispatch::call(
            &connected.cast::<IDispatch>().unwrap(),
            "WriteText",
            &[crate::comio::dispatch::bstr("event")],
        )
        .expect("WriteText");
        assert_eq!(unsafe { wrote.Anonymous.Anonymous.Anonymous.lVal }, 5);
        let got = crate::comio::dispatch::call(&stream, "ReadText", &[crate::comio::dispatch::i32v(16)])
            .expect("ReadText");
        assert_eq!(crate::comio::dispatch::as_string(&got).unwrap(), "event");

        unsafe { point.Unadvise(cookie) }.expect("Unadvise");
        // A cookie only works once; a stale one must not detach anything.
        assert!(unsafe { point.Unadvise(cookie) }.is_err());
    }

    /// An event the class does not raise is refused by the code a host uses
    /// to probe for optional events.
    #[test]
    fn an_unknown_source_interface_is_refused() {
        let listener = NamedPipeConnectionListener::new();
        let container: IConnectionPointContainer = listener.cast().unwrap();
        let stranger = GUID::from_u128(0x00000000_0000_0000_0000_00000000beef);
        let err = unsafe { container.FindConnectionPoint(&stranger) }
            .expect_err("not a source this class raises");
        assert_eq!(err.code(), CONNECT_E_NOCONNECTION);
    }

    /// A sink that is not dispatchable cannot be advised: the source is a
    /// dispinterface, so `IDispatch` is the only shape it can call.
    #[test]
    fn a_sink_without_idispatch_is_refused() {
        let listener = NamedPipeConnectionListener::new();
        let container: IConnectionPointContainer = listener.cast().unwrap();
        let point =
            unsafe { container.FindConnectionPoint(&IID_DLISTENER_EVENTS) }.expect("find");
        // The listener itself is an IUnknown, but advising it as a sink is
        // only sensible if it can be invoked — it can, so use something
        // that cannot: a bare connection point.
        let not_a_sink: IUnknown = point.cast().unwrap();
        let err = unsafe { point.Advise(&not_a_sink) }.expect_err("not dispatchable");
        assert_eq!(err.code(), CONNECT_E_CANNOTCONNECT);
    }
}
