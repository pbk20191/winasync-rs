//! The three listener classes: one per transport, each with the bind its
//! addressing has.

use super::*;

/// Generates one transport's COM type: the shared members on
/// [`IConnectionListener`], `Close` on `IClosable`, and inherent Rust
/// equivalents so Rust callers need no `cast`.
///
/// Every body delegates to [`ListenerCore`]; only the impl blocks repeat,
/// because `#[implement]` generates a per-type trait that cannot be blanket
/// implemented. The `_Impl` name is passed in rather than pasted, to avoid a
/// proc-macro dependency for one concatenation.
macro_rules! listener_impls {
    ($name:ident, $impl_name:ident) => {
        impl $name {
            /// Creates the listener. Nothing is created on disk or in the
            /// kernel until it is bound *and* has a handler.
            #[allow(clippy::new_without_default)]
            pub fn new() -> ComObject<$name> {
                ComObject::new($name {
                    core: ListenerCore::new(),
                    _module: ModuleRef::new(),
                })
            }

            /// Typed Rust equivalent of the vtable's `ConnectionReceived`.
            ///
            /// These three are `snake_case` while the binds below keep their
            /// WinRT-mirroring names, because only these three also exist on
            /// [`IConnectionListener`]: `ComObject` derefs through the
            /// generated `_Impl` type, so same-named inherent methods would
            /// be shadowed by the raw vtable ones. Separate names keep the
            /// typed surface and the ABI surface from colliding.
            pub fn connection_received(&self, handler: &ConnectionHandler) -> Result<i64> {
                self.core.connection_received(handler)
            }

            /// Typed Rust equivalent of the vtable's
            /// `RemoveConnectionReceived`.
            pub fn remove_connection_received(&self, token: i64) -> Result<()> {
                self.core.remove_connection_received(token)
            }

            /// Typed Rust equivalent of the vtable's `Information`.
            pub fn information(&self) -> Result<HSTRING> {
                self.core.information()
            }
        }

        // The generated binding borrows the handler as a `Ref`, carries the
        // token as the return value and the failure as an `Err`, so every
        // null check and hand-rolled `HRESULT` that used to live here is now
        // the projection's problem rather than ours.
        impl IConnectionListener_Impl for $impl_name {
            fn ConnectionReceived(
                &self,
                handler: windows_core::Ref<'_, ConnectionHandler>,
            ) -> Result<i64> {
                self.core.connection_received(handler.ok()?)
            }

            fn RemoveConnectionReceived(&self, token: i64) -> Result<()> {
                self.core.remove_connection_received(token)
            }

            fn Information(&self) -> Result<HSTRING> {
                self.core.information()
            }
        }

        impl IClosable_Impl for $impl_name {
            fn Close(&self) -> Result<()> {
                self.core.close()
            }
        }
    };
}

/// A listener that accepts named-pipe connections.
///
/// Binding creates nothing on its own: the accept chain — and with it any
/// pipe instance — starts only once the listener is both bound and has a
/// `ConnectionReceived` handler.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::NamedPipeConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::INamedPipeBind,
    bindings::Pbk20191::WinAsync::Comio::IListenerSecurity,
    bindings::Pbk20191::WinAsync::Comio::INamedPipeListenerSettings,
    windows::Foundation::IClosable
)]
pub struct NamedPipeConnectionListener {
    pub(super) core: ListenerCore,
    pub(super) _module: ModuleRef,
}

/// A listener that accepts AF_UNIX (Unix domain socket) connections.
///
/// The listen socket is created on the first accept, so binding alone leaves
/// nothing on disk.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::UnixConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::IUnixBind,
    bindings::Pbk20191::WinAsync::Comio::IListenerSecurity,
    windows::Foundation::IClosable
)]
pub struct UnixConnectionListener {
    pub(super) core: ListenerCore,
    pub(super) _module: ModuleRef,
}

/// A listener that accepts Hyper-V socket (AF_HYPERV) connections.
///
/// The listen socket is created on the first accept, as for AF_UNIX.
#[implement(
    bindings::Pbk20191::WinAsync::Comio::HyperVConnectionListener,
    bindings::Pbk20191::WinAsync::Comio::ITransport,
    bindings::Pbk20191::WinAsync::Comio::IHyperVBind,
    windows::Foundation::IClosable
)]
pub struct HyperVConnectionListener {
    pub(super) core: ListenerCore,
    pub(super) _module: ModuleRef,
}

// Which transport this listener is, answered from the same constant its
// connector answers with -- so the two halves of one transport cannot
// disagree about what it is called. The metadata puts this on `ITransport`
// rather than on anything named for a role, because binding, connecting and
// listening are three things one transport does.
impl ITransport_Impl for NamedPipeConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(crate::comio::pipe::TRANSPORT))
    }
}

impl ITransport_Impl for UnixConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(crate::comio::unix::TRANSPORT))
    }
}

impl ITransport_Impl for HyperVConnectionListener_Impl {
    fn Transport(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(crate::comio::hvsock::TRANSPORT))
    }
}

// Each listener's own bind, as metadata declares it. One interface per
// transport rather than one shared bind: the addressing genuinely differs,
// and this is what lets each listener offer only the bind it has.
impl INamedPipeBind_Impl for NamedPipeConnectionListener_Impl {
    fn BindAsync(&self, name: &HSTRING) -> Result<IAsyncAction> {
        self.bind(name)
    }
}

impl IUnixBind_Impl for UnixConnectionListener_Impl {
    fn BindAsync(&self, path: &HSTRING) -> Result<IAsyncAction> {
        self.bind(path)
    }
}

// Who may connect, on the two local transports. Not on the Hyper-V listener:
// who may reach a VM socket is configured with the VM.
impl IListenerSecurity_Impl for NamedPipeConnectionListener_Impl {
    fn SecurityDescriptor(&self) -> Result<HSTRING> {
        self.core.security_descriptor()
    }

    fn SetSecurityDescriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

// The metadata spells "no limit" as 0; the pipe API spells it 255. Both are
// accepted nowhere else, so the conversion lives here and only here.
impl INamedPipeListenerSettings_Impl for NamedPipeConnectionListener_Impl {
    fn MessageMode(&self) -> Result<bool> {
        Ok(self.core.pipe_options()?.message_mode)
    }

    fn SetMessageMode(&self, value: bool) -> Result<()> {
        self.core.update_pipe_options(|o| Ok(o.message_mode(value)))
    }

    fn AcceptRemoteClients(&self) -> Result<bool> {
        Ok(self.core.pipe_options()?.accept_remote_clients)
    }

    fn SetAcceptRemoteClients(&self, value: bool) -> Result<()> {
        self.core.update_pipe_options(|o| Ok(o.accept_remote_clients(value)))
    }

    fn MaxInstances(&self) -> Result<u32> {
        Ok(match self.core.pipe_options()?.max_instances {
            PIPE_UNLIMITED_INSTANCES => 0,
            n => n,
        })
    }

    fn SetMaxInstances(&self, value: u32) -> Result<()> {
        self.core.update_pipe_options(|o| match value {
            0 => Ok(o.max_instances(PIPE_UNLIMITED_INSTANCES)),
            1..PIPE_UNLIMITED_INSTANCES => Ok(o.max_instances(value)),
            _ => Err(Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "MaxInstances is 1 to 254, or 0 for no limit",
            )),
        })
    }

    fn InBufferSize(&self) -> Result<u32> {
        Ok(self.core.pipe_options()?.in_buffer)
    }

    fn SetInBufferSize(&self, value: u32) -> Result<()> {
        self.core
            .update_pipe_options(|o| Ok(NamedPipeOptions { in_buffer: value, ..o }))
    }

    fn OutBufferSize(&self) -> Result<u32> {
        Ok(self.core.pipe_options()?.out_buffer)
    }

    fn SetOutBufferSize(&self, value: u32) -> Result<()> {
        self.core
            .update_pipe_options(|o| Ok(NamedPipeOptions { out_buffer: value, ..o }))
    }
}

impl IListenerSecurity_Impl for UnixConnectionListener_Impl {
    fn SecurityDescriptor(&self) -> Result<HSTRING> {
        self.core.security_descriptor()
    }

    fn SetSecurityDescriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

// The string forms, which are what the metadata declares — see the RDL for
// why this endpoint crosses a projection as text rather than as the pair of
// GUIDs the transport takes. A malformed endpoint is `E_INVALIDARG`: the
// caller passed something that is not an address, which is exactly that.
impl IHyperVBind_Impl for HyperVConnectionListener_Impl {
    fn BindEndpointAsync(&self, endpoint: &HSTRING) -> Result<IAsyncAction> {
        let addr: HvSocketAddr = endpoint.to_string().parse().map_err(invalid_endpoint)?;
        self.bind_endpoint(&addr.vm_id, &addr.service_id)
    }

    fn BindServiceNameAsync(&self, serviceName: &HSTRING) -> Result<IAsyncAction> {
        let service = crate::common::hvaddr::service_id_from_str(&serviceName.to_string())
            .map_err(invalid_endpoint)?;
        self.bind_service_name(&service)
    }
}

/// E_INVALIDARG, carrying what the string should have looked like.
pub(super) fn invalid_endpoint(e: crate::common::hvaddr::HvAddrParseError) -> Error {
    Error::new(windows::Win32::Foundation::E_INVALIDARG, e.to_string())
}

// The struct declarations stay out of the macro: `#[implement]` is a
// proc-macro and does not survive being handed a `macro_rules` metavariable
// as an interface name. Only the bodies, which are identical, are generated.
listener_impls!(NamedPipeConnectionListener, NamedPipeConnectionListener_Impl);
listener_impls!(UnixConnectionListener, UnixConnectionListener_Impl);
listener_impls!(HyperVConnectionListener, HyperVConnectionListener_Impl);

impl NamedPipeConnectionListener {
    /// Binds to a pipe name such as `\\.\pipe\example`.
    ///
    /// No pipe instance exists until the accept chain starts. The first one
    /// claims the name, and from then on one listening instance is kept until
    /// `Close` (see `common::pipe`), so the name is never free for another
    /// server to take. A client may therefore connect while no handler is
    /// registered; it is delivered once one is.
    ///
    /// Uses the `INamedPipeListenerSettings` and `SecurityDescriptor` set so
    /// far, and fails with `E_INVALIDARG` if they cannot go together --
    /// remote clients without a descriptor.
    pub fn bind(&self, name: &HSTRING) -> Result<IAsyncAction> {
        let mut options = self.core.pipe_options()?;
        options.security = self.core.security();
        options.validate()?;
        let server = PipeServer::new(&name.to_string(), options);
        self.core
            .install_source(Arc::new(AcceptSource::Pipe(Arc::new(server))), name.clone())
    }

    /// Typed Rust equivalent of the vtable's `SecurityDescriptor` setter.
    pub fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

impl UnixConnectionListener {
    /// Binds to a filesystem path.
    ///
    /// The socket is bound lazily on the first accept, so a stale socket file
    /// surfaces as an error from whichever of this call or the first
    /// `ConnectionReceived` completes the gating condition.
    pub fn bind(&self, path: &HSTRING) -> Result<IAsyncAction> {
        self.core.install_source(
            Arc::new(AcceptSource::Unix(UnixSource {
                path: path.to_string(),
                security: self.core.security(),
                listener: Mutex::new(None),
            })),
            path.clone(),
        )
    }

    /// Typed Rust equivalent of the vtable's `SecurityDescriptor` setter.
    pub fn set_security_descriptor(&self, sddl: &HSTRING) -> Result<()> {
        self.core.set_security_descriptor(sddl)
    }
}

impl HyperVConnectionListener {
    /// Binds to one partition's `service_id` — the AF_HYPERV counterpart of
    /// `StreamSocketListener::BindEndpointAsync`, whose
    /// `(HostName, serviceName)` pair this mirrors: `vm_id` says *which
    /// partition*, `service_id` says *which endpoint inside it*.
    ///
    /// GUIDs rather than a `HostName` because WinRT has no way to spell this
    /// endpoint: `HostNameType` is `DomainName | Ipv4 | Ipv6 | Bluetooth`.
    ///
    /// Use `HV_GUID_WILDCARD` for `vm_id` to accept from any partition — or
    /// just call [`bind_service_name`](Self::bind_service_name).
        pub fn bind_endpoint(
        &self,
        vm_id: &windows_core::GUID,
        service_id: &windows_core::GUID,
    ) -> Result<IAsyncAction> {
        let addr = HvSocketAddr::new(*vm_id, *service_id);
        let label = HSTRING::from(addr.to_string());
        self.core.install_source(
            Arc::new(AcceptSource::HyperV(HyperVSource {
                addr,
                listener: Mutex::new(None),
            })),
            label,
        )
    }

    /// Binds `service_id` on *any* partition — the AF_HYPERV counterpart of
    /// `StreamSocketListener::BindServiceNameAsync`.
    ///
    /// The analogy is exact: where WinRT's version binds a service on every
    /// local adapter, this binds a service GUID with `HV_GUID_WILDCARD` as
    /// the VM id, accepting from any partition.
        pub fn bind_service_name(&self, service_id: &windows_core::GUID) -> Result<IAsyncAction> {
        self.bind_endpoint(&HV_GUID_WILDCARD, service_id)
    }
}
