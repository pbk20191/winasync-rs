//! `IDispatch`, so a scripting host can call these objects and not just
//! create them.
//!
//! [`super::activation`] makes `CoCreateInstance` work, which is where a
//! compiled caller stops needing help: C++ and C# declare the vtable and go.
//! A script cannot declare a vtable. PowerShell, VBScript, JScript and VBA
//! reach an object only through `IDispatch` — names resolved to DISPIDs at
//! run time, arguments passed as `VARIANT`s. Without it
//! `New-Object -ComObject Rustwinui3loop.NamedPipeClient` succeeds and then
//! every method call fails.
//!
//! # Names, and where the type library comes from
//!
//! `GetIDsOfNames` resolves against a table in this module rather than a
//! compiled `.tlb`, so late binding works whether or not anything has been
//! registered. [`super::typelib`] then *generates* the library from these
//! same tables, which is what `GetTypeInfo` hands back — so early binding
//! and late binding cannot describe a member differently, and the build
//! still needs no IDL and no `midl.exe`.
//!
//! # Frozen, as of 2026-11-09
//!
//! This layer is kept but no longer developed. comio's target consumers are
//! WinRT projections — Python, Node, .NET scripting — which read a `.winmd`
//! and never ask an object for `IDispatch`. What this reaches instead is the
//! classic script hosts, and those cannot consume WinRT at any price: they
//! predate it and only speak `IDispatch`. So it stays for them; it just stops
//! growing. `docs/comio-com-winrt-status.md` carries the reasoning and lists
//! what the decision dropped — including an operation object that would have
//! gone here.
//!
//! # A synchronous surface, deliberately
//!
//! The vtables hand back `IAsyncOperation` and `IAsyncAction`. A script has
//! no way to await one — it cannot build the completion delegate, and the
//! generic interfaces are not dispatchable either. So the dispatch surface
//! is the blocking shape of the same operations: `Connect` is `ConnectAsync`
//! already waited on, and `Accept` waits on a queue this module fills.
//!
//! That is a real difference in semantics, not a rename, which is why the
//! names differ from the vtable's. A compiled caller keeps the async
//! surface; a script gets the one it can use. Nothing is removed from
//! either.
//!
//! # Events become a queue
//!
//! The listeners are event-driven: `ConnectionReceived` takes a
//! `TypedEventHandler`, which no script can construct. The dispatch `Bind`
//! therefore registers a handler of this module's own and pushes each
//! accepted stream onto a queue that `Accept(timeoutMs)` pops.
//!
//! [`super::connpt`] later made the event reachable properly, through a
//! connection point a host advises a sink on. `Accept` stays, and not only
//! for compatibility: a script that simply wants the next connection says
//! so in one line, and PowerShell cannot use the sink path at all for
//! reasons that module documents.
//!
//! This also fixes an ordering problem that would otherwise make the whole
//! thing unusable from a single-threaded script. Accepting is gated on a
//! handler existing, and a named-pipe client cannot connect before the
//! server has created an instance — so a script that called a blocking
//! `Accept` before connecting would deadlock, and one that connected first
//! would find nothing listening. Registering at `Bind` means the chain is
//! running from then on and connections queue up, so `Bind` → connect →
//! `Accept` works in one thread.

use std::sync::mpsc;
use windows::Win32::Foundation::{
    DISP_E_BADPARAMCOUNT, DISP_E_MEMBERNOTFOUND, DISP_E_TYPEMISMATCH, DISP_E_UNKNOWNNAME, E_POINTER,
};
use windows::Win32::System::Com::{DISPPARAMS, IDispatch};
use windows::Win32::System::Ole::DISPID_UNKNOWN;
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VT_DISPATCH, VT_I4, VT_UI4,
    VariantChangeType,
};
use windows_core::{AgileReference, BSTR, GUID, HRESULT, IInspectable, Interface, PCWSTR, Result};

/// One dispatchable member: the name a script writes and the DISPID it
/// resolves to.
///
/// DISPIDs are frozen per class for the same reason the CLSIDs are: a host
/// that caches `GetIDsOfNames` across runs — most of them do — would call
/// the wrong member after a renumber.
pub(crate) struct Member {
    pub name: &'static str,
    pub dispid: i32,
    /// In source order, as a caller writes them.
    pub params: &'static [Param],
    pub returns: Vt,
}

/// One declared parameter. The name is what a host shows in a signature and
/// what named-argument binding would resolve against.
pub(crate) struct Param {
    pub name: &'static str,
    pub vt: Vt,
}

/// The parameter and return types this surface uses, which is all a type
/// library needs to describe it.
///
/// Narrow on purpose: a wider set would be types no member here returns,
/// and every entry has to be something [`super::typelib`] can emit and the
/// VARIANT helpers above can produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Vt {
    /// No return value.
    None,
    Text,
    Count,
    /// Another dispatchable object.
    Object,
    /// A GUID, which a script writes as text and [`as_guid`] parses — so it
    /// is a `BSTR` on the wire and only the name says more.
    Guid,
}

impl Vt {
    /// The `VARENUM` a type library records for this.
    pub(crate) fn varenum(self) -> windows::Win32::System::Variant::VARENUM {
        use windows::Win32::System::Variant::{VT_BSTR, VT_DISPATCH, VT_I4, VT_VOID};
        match self {
            Vt::None => VT_VOID,
            Vt::Text | Vt::Guid => VT_BSTR,
            Vt::Count => VT_I4,
            Vt::Object => VT_DISPATCH,
        }
    }
}

/// Declares a class's dispatch surface.
///
/// Implemented on the concrete type rather than on the `#[implement]`-
/// generated `_Impl` wrapper, so the bodies see the real fields.
pub(crate) trait Dispatchable {
    /// What `GetIDsOfNames` resolves against. Searched case-insensitively:
    /// VBScript and PowerShell do not preserve the case a user typed.
    const MEMBERS: &'static [Member];

    /// The dispinterface in [`super::typelib`] that describes [`MEMBERS`],
    /// and what `GetTypeInfo` hands back.
    ///
    /// [`MEMBERS`]: Dispatchable::MEMBERS
    const DISPINTERFACE: GUID;

    /// Runs one member. `args` is in source order — see [`invoke`], which
    /// has already undone `DISPPARAMS`' reversal.
    fn invoke_member(&self, dispid: i32, args: &[VARIANT]) -> Result<VARIANT>;
}

/// `IDispatch::GetIDsOfNames` over a [`Dispatchable::MEMBERS`] table.
///
/// Named arguments are not supported, so a call that asks for more than the
/// member name itself gets `DISP_E_UNKNOWNNAME` for the extras — which is
/// what the contract requires, and what makes a host fall back to
/// positional arguments rather than fail outright.
///
/// # Safety
///
/// `names` must point to `count` valid `PCWSTR`s and `ids` to `count`
/// writable `i32`s, as the COM caller guarantees.
pub(crate) unsafe fn ids_of_names(
    members: &'static [Member],
    names: *const PCWSTR,
    count: u32,
    ids: *mut i32,
) -> Result<()> {
    if names.is_null() || ids.is_null() {
        return Err(windows_core::Error::from_hresult(E_POINTER));
    }
    let mut unknown = false;
    for i in 0..count as usize {
        let wanted = unsafe { (*names.add(i)).to_string() }.unwrap_or_default();
        // Only index 0 is a member name; the rest would be named arguments.
        let found = (i == 0)
            .then(|| {
                members
                    .iter()
                    .find(|m| m.name.eq_ignore_ascii_case(&wanted))
            })
            .flatten();
        unsafe {
            *ids.add(i) = match found {
                Some(m) => m.dispid,
                None => {
                    unknown = true;
                    DISPID_UNKNOWN
                }
            }
        };
    }
    if unknown {
        return Err(windows_core::Error::from_hresult(DISP_E_UNKNOWNNAME));
    }
    Ok(())
}

/// `IDispatch::Invoke` over a [`Dispatchable`].
///
/// Flags are not checked. A dispatch-only object with no properties has
/// nothing to distinguish `DISPATCH_METHOD` from `DISPATCH_PROPERTYGET`, and
/// hosts disagree about which to send for a zero-argument member —
/// PowerShell reads `$obj.Transport` as a property get and `$obj.Connect(x)`
/// as a method, while others send both bits at once. Accepting any of them
/// is what makes the same member work from every host.
///
/// # Safety
///
/// `params` and `result`, where non-null, must be valid as the COM caller
/// guarantees.
pub(crate) unsafe fn invoke<T: Dispatchable>(
    this: &T,
    dispid: i32,
    params: *const DISPPARAMS,
    result: *mut VARIANT,
) -> Result<()> {
    let Some(member) = T::MEMBERS.iter().find(|m| m.dispid == dispid) else {
        return Err(windows_core::Error::from_hresult(DISP_E_MEMBERNOTFOUND));
    };

    // `DISPPARAMS` holds arguments in reverse: rgvarg[0] is the *last* one.
    // Undoing that here keeps every member body reading left to right.
    let args: Vec<VARIANT> = match unsafe { params.as_ref() } {
        None => Vec::new(),
        Some(p) => {
            if p.cNamedArgs != 0 {
                // Named arguments would need a type library to resolve.
                return Err(windows_core::Error::from_hresult(DISP_E_UNKNOWNNAME));
            }
            let n = p.cArgs as usize;
            if n != 0 && p.rgvarg.is_null() {
                return Err(windows_core::Error::from_hresult(E_POINTER));
            }
            (0..n)
                .rev()
                .map(|i| unsafe { (*p.rgvarg.add(i)).clone() })
                .collect()
        }
    };

    // Arity is checked here, against the same declaration the type library
    // is generated from, rather than in each member body — so a signature
    // cannot be advertised in one place and enforced differently in another.
    if args.len() != member.params.len() {
        return Err(windows_core::Error::from_hresult(DISP_E_BADPARAMCOUNT));
    }

    let value = this.invoke_member(dispid, &args)?;
    // A caller that does not want the result passes null; the value is then
    // dropped here rather than leaked.
    if !result.is_null() {
        unsafe { *result = value };
    }
    Ok(())
}

/// One [`Member`], written the way a signature reads.
///
/// `member!("Connect", 2, ("Name": Text) -> Object)` — the name a script
/// calls, the frozen DISPID, the parameters, and what comes back. The same
/// line feeds `Invoke`'s arity check and the type library's `FUNCDESC`.
macro_rules! member {
    ($name:literal, $id:expr, ($($pn:literal : $pv:ident),* $(,)?) -> $ret:ident) => {
        $crate::comio::dispatch::Member {
            name: $name,
            dispid: $id,
            params: &[$(
                $crate::comio::dispatch::Param {
                    name: $pn,
                    vt: $crate::comio::dispatch::Vt::$pv,
                },
            )*],
            returns: $crate::comio::dispatch::Vt::$ret,
        }
    };
}

pub(crate) use member;

/// Generates the `IDispatch` vtable bodies for a `#[implement]`ed type.
///
/// Only the bodies: as with `listener_impls!`, `#[implement]` is a
/// proc-macro and cannot be handed an interface name through a
/// `macro_rules` metavariable, so each type still lists `IDispatch` itself.
///
/// Two forms, because members need different things to be in scope:
///
/// - `impl_dispatch!(Type, Type_Impl)` when the member bodies call the
///   type's own inherent methods, which is the usual case. [`Dispatchable`]
///   goes on `Type` and this bridges it to the wrapper.
/// - `impl_dispatch!(wrapper Type_Impl)` when they need the generated
///   wrapper's WinRT interface implementations — `ReadAsync` and friends
///   live on `Type_Impl`, not on `Type`. [`Dispatchable`] goes on
///   `Type_Impl`.
macro_rules! impl_dispatch {
    ($ty:ty, $impl_ty:ident) => {
        // Members are written against the inner type; the vtable only ever
        // has the wrapper, so forward across the one `Deref`.
        impl $crate::comio::dispatch::Dispatchable for $impl_ty {
            const MEMBERS: &'static [$crate::comio::dispatch::Member] =
                <$ty as $crate::comio::dispatch::Dispatchable>::MEMBERS;

            const DISPINTERFACE: windows_core::GUID =
                <$ty as $crate::comio::dispatch::Dispatchable>::DISPINTERFACE;

            fn invoke_member(
                &self,
                dispid: i32,
                args: &[windows::Win32::System::Variant::VARIANT],
            ) -> windows_core::Result<windows::Win32::System::Variant::VARIANT> {
                <$ty as $crate::comio::dispatch::Dispatchable>::invoke_member(
                    &**self, dispid, args,
                )
            }
        }

        $crate::comio::dispatch::impl_dispatch!(wrapper $impl_ty);
    };

    (wrapper $impl_ty:ident) => {
        // The raw-pointer parameters come from `IDispatch_Impl`, whose
        // signatures are generated by `windows` and are not `unsafe fn` —
        // so this cannot be marked unsafe the way clippy would like. The
        // pointers are validated before use in `ids_of_names` / `invoke`.
        #[allow(clippy::not_unsafe_ptr_arg_deref)]
        impl windows::Win32::System::Com::IDispatch_Impl for $impl_ty {
            /// One: the library `super::typelib` generates and registers.
            fn GetTypeInfoCount(&self) -> windows_core::Result<u32> {
                Ok(1)
            }

            /// The dispinterface describing this class.
            ///
            /// Fails with `TYPE_E_LIBNOTREGISTERED` when the library has not
            /// been registered — which a host treats as "no type info" and
            /// falls back to pure late binding, exactly as before this
            /// existed.
            fn GetTypeInfo(
                &self,
                index: u32,
                _lcid: u32,
            ) -> windows_core::Result<windows::Win32::System::Com::ITypeInfo> {
                if index != 0 {
                    return Err(windows_core::Error::from_hresult(
                        windows::Win32::Foundation::DISP_E_BADINDEX,
                    ));
                }
                $crate::comio::typelib::type_info(
                    &<$impl_ty as $crate::comio::dispatch::Dispatchable>::DISPINTERFACE,
                )
            }

            fn GetIDsOfNames(
                &self,
                _riid: *const windows_core::GUID,
                names: *const windows_core::PCWSTR,
                count: u32,
                _lcid: u32,
                ids: *mut i32,
            ) -> windows_core::Result<()> {
                unsafe {
                    $crate::comio::dispatch::ids_of_names(
                        <$impl_ty as $crate::comio::dispatch::Dispatchable>::MEMBERS,
                        names,
                        count,
                        ids,
                    )
                }
            }

            fn Invoke(
                &self,
                dispid: i32,
                _riid: *const windows_core::GUID,
                _lcid: u32,
                _flags: windows::Win32::System::Com::DISPATCH_FLAGS,
                params: *const windows::Win32::System::Com::DISPPARAMS,
                result: *mut windows::Win32::System::Variant::VARIANT,
                _excep: *mut windows::Win32::System::Com::EXCEPINFO,
                _arg_err: *mut u32,
            ) -> windows_core::Result<()> {
                unsafe { $crate::comio::dispatch::invoke(self, dispid, params, result) }
            }
        }
    };
}

pub(crate) use impl_dispatch;

// ---------------------------------------------------------------------------
// VARIANT helpers
//
// `windows` exposes `VARIANT` as the raw tagged union it is on the wire, so
// these are the conversions every member body would otherwise repeat. They
// are deliberately narrow: this surface passes strings, counts and objects,
// and nothing else.
// ---------------------------------------------------------------------------

/// Builds a VARIANT of `vt` with `value` written into the union.
fn raw(vt: windows::Win32::System::Variant::VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: core::mem::ManuallyDrop::new(VARIANT_0_0 {
                vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: value,
            }),
        },
    }
}

/// `VT_EMPTY` — what a member with nothing to return hands back.
pub(crate) fn empty() -> VARIANT {
    VARIANT::default()
}

/// `VT_BSTR`. Ownership of the allocation moves into the VARIANT, which the
/// caller (the host) clears.
pub(crate) fn bstr(value: &str) -> VARIANT {
    raw(
        VT_BSTR,
        VARIANT_0_0_0 {
            bstrVal: core::mem::ManuallyDrop::new(BSTR::from(value)),
        },
    )
}

/// `VT_I4`.
pub(crate) fn i32v(value: i32) -> VARIANT {
    raw(VT_I4, VARIANT_0_0_0 { lVal: value })
}

/// `VT_DISPATCH`, the only way to hand a script another object it can call.
///
/// Fails with `DISP_E_TYPEMISMATCH` rather than returning something
/// uncallable if the object has no `IDispatch` — which would be a bug here,
/// not a caller error, but silently handing back an opaque object is worse.
pub(crate) fn object(value: &IInspectable) -> Result<VARIANT> {
    let disp: IDispatch = value
        .cast()
        .map_err(|_| windows_core::Error::from_hresult(DISP_E_TYPEMISMATCH))?;
    Ok(raw(
        VT_DISPATCH,
        VARIANT_0_0_0 {
            pdispVal: core::mem::ManuallyDrop::new(Some(disp)),
        },
    ))
}

/// Reads an argument as a string, coercing whatever the host sent.
///
/// Scripts are loosely typed and a host may pass a number, a date or a
/// `VT_VARIANT|VT_BYREF` wrapper where a string is meant. `VariantChangeType`
/// is the coercion the rest of COM uses, so using it here means these
/// members accept exactly what every other automation object accepts.
pub(crate) fn as_string(value: &VARIANT) -> Result<String> {
    let mut converted = VARIANT::default();
    unsafe { VariantChangeType(&mut converted, value, Default::default(), VT_BSTR) }
        .map_err(|_| windows_core::Error::from_hresult(DISP_E_TYPEMISMATCH))?;
    let text = unsafe {
        let inner = &converted.Anonymous.Anonymous;
        inner.Anonymous.bstrVal.to_string()
    };
    unsafe { windows::Win32::System::Variant::VariantClear(&mut converted) }?;
    Ok(text)
}

/// Reads an argument as an unsigned count.
pub(crate) fn as_u32(value: &VARIANT) -> Result<u32> {
    let mut converted = VARIANT::default();
    unsafe { VariantChangeType(&mut converted, value, Default::default(), VT_UI4) }
        .map_err(|_| windows_core::Error::from_hresult(DISP_E_TYPEMISMATCH))?;
    let n = unsafe { converted.Anonymous.Anonymous.Anonymous.ulVal };
    Ok(n)
}

/// Reads an argument as a GUID, accepting the braced and bare spellings a
/// script is equally likely to produce.
pub(crate) fn as_guid(value: &VARIANT) -> Result<GUID> {
    let text = as_string(value)?;
    let trimmed = text.trim().trim_start_matches('{').trim_end_matches('}');
    GUID::try_from(trimmed).map_err(|_| windows_core::Error::from_hresult(DISP_E_TYPEMISMATCH))
}

// ---------------------------------------------------------------------------
// The accept queue behind the listeners' `Accept`
// ---------------------------------------------------------------------------

/// Turns `ConnectionReceived` into something a script can pull from.
///
/// Created on the first dispatch `Bind` and left registered: see the module
/// docs for why the handler goes on at bind time rather than at the first
/// `Accept`.
pub(crate) struct AcceptQueue {
    rx: mpsc::Receiver<AgileReference<IInspectable>>,
    /// Kept so the handler can be removed when the listener closes; unused
    /// otherwise, since the queue lives as long as the listener.
    #[allow(dead_code)]
    token: i64,
}

impl AcceptQueue {
    pub(crate) fn new(rx: mpsc::Receiver<AgileReference<IInspectable>>, token: i64) -> Self {
        Self { rx, token }
    }
}

/// Calls a member by name, the way a scripting host does.
///
/// `GetIDsOfNames` then `Invoke`, including the `DISPPARAMS` argument
/// reversal — this is the caller-side half of the contract, so using it in
/// tests means they exercise the same path PowerShell does rather than a
/// convenient shortcut around it.
///
/// `pub(crate)` rather than test-only because the `activation` integration
/// test drives objects COM created through this too.
pub(crate) fn call(object: &IDispatch, name: &str, args: &[VARIANT]) -> Result<VARIANT> {
    let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
    let names = [PCWSTR(wide.as_ptr())];
    let mut dispid = 0i32;
    unsafe { object.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0, &mut dispid) }?;

    // Reversed, as a host sends them.
    let mut reversed: Vec<VARIANT> = args.iter().rev().cloned().collect();
    let params = DISPPARAMS {
        rgvarg: reversed.as_mut_ptr(),
        rgdispidNamedArgs: core::ptr::null_mut(),
        cArgs: reversed.len() as u32,
        cNamedArgs: 0,
    };
    let mut result = VARIANT::default();
    unsafe {
        object.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            windows::Win32::System::Com::DISPATCH_METHOD,
            &params,
            Some(&mut result),
            None,
            None,
        )
    }?;
    Ok(result)
}

impl AcceptQueue {
    /// Pops one accepted stream, waiting up to `timeout_ms`.
    ///
    /// A timeout is an error rather than an empty success because a VARIANT
    /// has no "nothing arrived" that a script could tell apart from a
    /// stream — `On Error` / `try` is the only branch available to it.
    pub(crate) fn pop(&self, timeout_ms: u32) -> Result<IInspectable> {
        let timeout = std::time::Duration::from_millis(timeout_ms as u64);
        self.rx
            .recv_timeout(timeout)
            .map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => {
                windows_core::Error::from_hresult(HRESULT::from_win32(
                    windows::Win32::Foundation::WAIT_TIMEOUT.0,
                ))
            }
            // The sender is gone only if the listener was closed.
            mpsc::RecvTimeoutError::Disconnected => {
                windows_core::Error::from_hresult(windows::Win32::Foundation::RO_E_CLOSED)
            }
            })
            .and_then(|agile| agile.resolve())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::{HvStreamSocket, NamedPipeClient, NamedPipeConnectionListener};
    use windows::Win32::Foundation::{DISP_E_BADPARAMCOUNT, DISP_E_UNKNOWNNAME};

    /// `Result::expect_err` needs `Debug` on the Ok type, and a raw
    /// `VARIANT` has none — nor should it, being a union.
    fn expect_err(r: Result<VARIANT>, what: &str) -> windows_core::Error {
        match r {
            Ok(_) => panic!("{what}: expected an error, got a value"),
            Err(e) => e,
        }
    }

    /// Reads a `VT_BSTR` result back out.
    fn text(v: &VARIANT) -> String {
        as_string(v).expect("result is not a string")
    }

    /// The whole point, in one test: a late-bound caller binds a listener,
    /// connects a client, and talks over the stream it was handed — every
    /// step by name through `IDispatch`, never through a vtable.
    #[test]
    fn a_late_bound_caller_can_run_a_whole_connection() {
        let name = format!(r"\\.\pipe\comio_dispatch_{}", std::process::id());

        let listener: IDispatch = NamedPipeConnectionListener::new().cast().unwrap();
        call(&listener, "Bind", &[bstr(&name)]).expect("Bind");

        // `Information` reports what the bind resolved to.
        assert_eq!(text(&call(&listener, "Information", &[]).expect("Information")), name);

        let client: IDispatch = NamedPipeClient::new().cast().unwrap();
        assert_eq!(text(&call(&client, "Transport", &[]).expect("Transport")), "namedpipe");

        // Connect blocks and hands back a stream — as another IDispatch,
        // which is the only kind of object a script could do anything with.
        let connected = call(&client, "Connect", &[bstr(&name)]).expect("Connect");
        let client_stream = dispatch_of(&connected);

        // The connection is already queued: `Bind` registered the handler,
        // so nothing had to be waiting at the moment the client connected.
        let accepted = call(&listener, "Accept", &[i32v(2000)]).expect("Accept");
        let server_stream = dispatch_of(&accepted);

        let wrote = call(&client_stream, "WriteText", &[bstr("hello dispatch")]).expect("WriteText");
        assert_eq!(unsafe { wrote.Anonymous.Anonymous.Anonymous.lVal }, 14);

        let got = call(&server_stream, "ReadText", &[i32v(64)]).expect("ReadText");
        assert_eq!(text(&got), "hello dispatch");

        // And back the other way, so this is not one-directional luck.
        call(&server_stream, "WriteText", &[bstr("ack")]).expect("WriteText");
        assert_eq!(text(&call(&client_stream, "ReadText", &[i32v(64)]).expect("ReadText")), "ack");

        call(&client_stream, "Close", &[]).expect("Close");
        call(&listener, "Close", &[]).expect("Close");
    }

    /// Pulls the `IDispatch` out of a `VT_DISPATCH` result.
    fn dispatch_of(v: &VARIANT) -> IDispatch {
        unsafe {
            let inner = &v.Anonymous.Anonymous;
            assert_eq!(inner.vt, VT_DISPATCH, "member did not return an object");
            inner.Anonymous.pdispVal.as_ref().expect("null IDispatch").clone()
        }
    }

    /// Name resolution is what a host does first and caches; these are the
    /// cases it has to get right.
    #[test]
    fn names_resolve_the_way_a_host_expects() {
        let client: IDispatch = NamedPipeClient::new().cast().unwrap();

        // Case-insensitive: VBScript and PowerShell do not preserve case.
        let lower = call(&client, "transport", &[]).expect("lowercase name");
        assert_eq!(text(&lower), "namedpipe");

        // An unknown name is DISP_E_UNKNOWNNAME, not a crash or a wrong
        // member — a host probes for optional members this way.
        let err = expect_err(call(&client, "NoSuchMember", &[]), "unknown name");
        assert_eq!(err.code(), DISP_E_UNKNOWNNAME);

        // Wrong arity is reported rather than read past the argument array.
        let err = expect_err(call(&client, "Transport", &[bstr("extra")]), "arity");
        assert_eq!(err.code(), DISP_E_BADPARAMCOUNT);
    }

    /// Dispatch does not quietly widen what a class accepts: the Hyper-V
    /// connector still wants two GUIDs, in either spelling a script writes.
    #[test]
    fn guid_arguments_accept_both_spellings_and_reject_junk() {
        let hv: IDispatch = HvStreamSocket::new().cast().unwrap();
        assert_eq!(text(&call(&hv, "Transport", &[]).expect("Transport")), "hyperv");

        // Not a GUID at all.
        let err = expect_err(
            call(&hv, "Connect", &[bstr("not-a-guid"), bstr("also-not")]),
            "junk GUID",
        );
        assert_eq!(err.code(), DISP_E_TYPEMISMATCH);

        // Braced and bare both parse; the connect itself then fails because
        // nothing is listening on that endpoint, which is a different error
        // and is what proves the arguments got through.
        for (vm, service) in [
            ("{00000000-0000-0000-0000-000000000000}", "{00000000-0000-0000-0000-000000000000}"),
            ("00000000-0000-0000-0000-000000000000", "00000000-0000-0000-0000-000000000000"),
        ] {
            let err = expect_err(
                call(&hv, "Connect", &[bstr(vm), bstr(service)]),
                "nothing is listening there",
            );
            assert_ne!(err.code(), DISP_E_TYPEMISMATCH, "{vm} should have parsed");
        }
    }

    /// `Accept` before `Bind` has nothing to wait on, and says so at once
    /// instead of burning the timeout.
    #[test]
    fn accept_before_bind_fails_immediately() {
        let listener: IDispatch = NamedPipeConnectionListener::new().cast().unwrap();
        let started = std::time::Instant::now();
        let err = expect_err(call(&listener, "Accept", &[i32v(30_000)]), "not bound");
        assert_eq!(
            err.code(),
            windows::Win32::Foundation::E_ILLEGAL_METHOD_CALL
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
