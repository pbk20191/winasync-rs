//! The type library, built from the dispatch tables rather than from IDL.
//!
//! [`super::dispatch`] gets a script calling these objects with nothing but
//! names. Two things still need more than names:
//!
//! - **Events.** A host that wants to be *called back* has to know which
//!   source interface to hand a sink to. That is what a coclass's
//!   `[default, source]` declaration says, and there is nowhere else to say
//!   it. Without this module, [`super::connpt`] would have connection points
//!   nothing could discover.
//! - **Early binding.** VBA can only write `Dim x As NamedPipeClient`, and
//!   offer IntelliSense for it, if a library describes the members.
//!
//! # Why not IDL and `midl`
//!
//! The usual way to get a `.tlb` is to write the whole surface a second time
//! in IDL and compile it. That would put every member name, DISPID and
//! signature in two places with nothing tying them together, and it would
//! make the build depend on `midl.exe` — which in turn needs `cl.exe` as its
//! preprocessor and the SDK's `oaidl.idl` on an include path.
//!
//! Instead the library is generated through `ICreateTypeLib2` directly from
//! `Dispatchable::MEMBERS`, the same tables `Invoke`
//! dispatches on and
//! checks arity against. A member cannot be described differently from how
//! it behaves, because there is only one description. The build stays
//! toolchain-free.
//!
//! # Where the file lives
//!
//! `%LOCALAPPDATA%\rustwinui3loop\rustwinui3loop.tlb`, written by
//! `DllRegisterServer` and removed by `DllUnregisterServer`.
//!
//! The conventional home is a `TYPELIB` resource inside the DLL, which would
//! mean generating the `.tlb` at build time and embedding it. That trades
//! this module's single-source property for a build step, and gains nothing
//! here: the library is only ever read after registration, and registration
//! is already per-user and already writes to a place it is allowed to.
//! `%LOCALAPPDATA%` is always writable, which `regsvr32` on a DLL under
//! `Program Files` would not be.

use super::dispatch::{Member, Param, Vt};
use std::path::PathBuf;
use windows::Win32::System::Com::{
    CC_STDCALL, ELEMDESC, ELEMDESC_0, FUNC_DISPATCH, FUNCDESC, IDLDESC, IMPLTYPEFLAG_FDEFAULT,
    IMPLTYPEFLAG_FSOURCE, INVOKE_FUNC, ITypeInfo, ITypeLib, SYS_WIN64, TKIND_COCLASS,
    TKIND_DISPATCH, TYPEDESC,
};
use windows::Win32::System::Ole::{
    CreateTypeLib2, ICreateTypeInfo, LoadRegTypeLib, LoadTypeLibEx, PARAMDESC, PARAMFLAG_FIN,
    REGKIND_NONE, RegisterTypeLibForUser, TYPEFLAG_FCANCREATE, TYPEFLAG_FDISPATCHABLE,
    UnRegisterTypeLibForUser,
};
use windows_core::{GUID, HSTRING, Interface, PCWSTR, Result};

/// The library's own identity. Frozen, like every other GUID here: a host
/// that has early-bound against it records this.
pub const LIBID: GUID = GUID::from_u128(0x1bec2bdb_a2e5_4344_8ce3_c1327ad6f974);
pub const VERSION: (u16, u16) = (1, 0);
const LIBRARY_NAME: &str = "Rustwinui3loop";
/// `LOCALE_NEUTRAL`. See `build` for why this is set rather than defaulted.
const LCID_NEUTRAL: u32 = 0;

// One dispinterface per dispatchable class, plus the one listeners fire.
// A dispinterface rather than a dual interface: these members exist only
// through `Invoke`, there is no vtable behind them, and claiming one in the
// library would be a promise `QueryInterface` could not keep.
pub const IID_DNAMED_PIPE_LISTENER: GUID =
    GUID::from_u128(0x39368477_a653_47ef_abd2_8c018c3eda6b);
pub const IID_DUNIX_LISTENER: GUID = GUID::from_u128(0xfae6c0ab_c9d6_4689_8610_06226e2c05f1);
pub const IID_DHYPERV_LISTENER: GUID = GUID::from_u128(0xfb2ddc5f_90e3_480a_9621_2315a65b7e8f);
pub const IID_DNAMED_PIPE_CLIENT: GUID = GUID::from_u128(0xb5e7373d_6e5e_4216_9a2b_8ac07a78df60);
pub const IID_DUNIX_SOCKET: GUID = GUID::from_u128(0xd42c5a97_ff8f_4920_a638_ae6b021f372a);
pub const IID_DHV_SOCKET: GUID = GUID::from_u128(0xaf2a7dec_4ce2_4e04_90a5_3e98a42b471f);
pub const IID_DSTREAM: GUID = GUID::from_u128(0x5c2acba9_97e2_4ee0_8eb3_75e2d789708b);

/// The source interface every listener fires. One shared interface rather
/// than one per transport: the event carries a stream, and a stream is a
/// stream whichever transport accepted it.
pub const IID_DLISTENER_EVENTS: GUID = GUID::from_u128(0xe9a08029_2874_4189_a76c_63625dc74ede);

/// `ConnectionReceived(Stream)`. Frozen — a sink's `Invoke` is dispatched on
/// this.
pub const DISPID_CONNECTION_RECEIVED: i32 = 1;

/// The one event listeners raise.
pub(crate) const LISTENER_EVENTS: &[Member] = &[Member {
    name: "ConnectionReceived",
    dispid: DISPID_CONNECTION_RECEIVED,
    params: &[Param {
        name: "Stream",
        vt: Vt::Object,
    }],
    returns: Vt::None,
}];

/// Every dispinterface, for the registry entries that make one resolvable
/// by IID. Paired with its name, which becomes the key's default value.
pub(crate) const DISPINTERFACE_IDS: &[(&str, &GUID)] = &[
    ("DNamedPipeListener", &IID_DNAMED_PIPE_LISTENER),
    ("DUnixListener", &IID_DUNIX_LISTENER),
    ("DHyperVListener", &IID_DHYPERV_LISTENER),
    ("DNamedPipeClient", &IID_DNAMED_PIPE_CLIENT),
    ("DUnixStreamSocket", &IID_DUNIX_SOCKET),
    ("DHvStreamSocket", &IID_DHV_SOCKET),
    ("DStream", &IID_DSTREAM),
    ("DListenerEvents", &IID_DLISTENER_EVENTS),
];

/// A dispinterface as the library records it.
struct DispInterface {
    name: &'static str,
    iid: GUID,
    members: &'static [Member],
}

/// A coclass: what `CoCreateInstance` makes, and the interfaces a host
/// should look for on it.
struct CoClass {
    name: &'static str,
    clsid: GUID,
    /// The `[default]` interface — what an early-bound variable is typed as.
    default_iface: GUID,
    /// The `[default, source]` interface, for classes that raise events.
    source_iface: Option<GUID>,
}

/// Everything the library declares.
///
/// Member tables are borrowed from the classes themselves, so this list
/// says only which class is described by which interface — never what the
/// members are.
fn dispinterfaces() -> Vec<DispInterface> {
    use super::dispatch::Dispatchable;
    use super::{
        HvStreamSocket, HyperVConnectionListener, NamedPipeClient, NamedPipeConnectionListener,
        UnixConnectionListener, UnixStreamSocket,
    };
    vec![
        DispInterface {
            name: "DNamedPipeListener",
            iid: IID_DNAMED_PIPE_LISTENER,
            members: NamedPipeConnectionListener::MEMBERS,
        },
        DispInterface {
            name: "DUnixListener",
            iid: IID_DUNIX_LISTENER,
            members: UnixConnectionListener::MEMBERS,
        },
        DispInterface {
            name: "DHyperVListener",
            iid: IID_DHYPERV_LISTENER,
            members: HyperVConnectionListener::MEMBERS,
        },
        DispInterface {
            name: "DNamedPipeClient",
            iid: IID_DNAMED_PIPE_CLIENT,
            members: NamedPipeClient::MEMBERS,
        },
        DispInterface {
            name: "DUnixStreamSocket",
            iid: IID_DUNIX_SOCKET,
            members: UnixStreamSocket::MEMBERS,
        },
        DispInterface {
            name: "DHvStreamSocket",
            iid: IID_DHV_SOCKET,
            members: HvStreamSocket::MEMBERS,
        },
        DispInterface {
            name: "DStream",
            iid: IID_DSTREAM,
            members: <super::stream::OverlappedDuplexStream_Impl as Dispatchable>::MEMBERS,
        },
        DispInterface {
            name: "DListenerEvents",
            iid: IID_DLISTENER_EVENTS,
            members: LISTENER_EVENTS,
        },
    ]
}

/// The coclasses, which must line up with [`super::activation::CLASSES`] —
/// a test asserts they do.
fn coclasses() -> Vec<CoClass> {
    use super::activation as act;
    vec![
        CoClass {
            name: "NamedPipeConnectionListener",
            clsid: act::CLSID_NAMED_PIPE_CONNECTION_LISTENER,
            default_iface: IID_DNAMED_PIPE_LISTENER,
            source_iface: Some(IID_DLISTENER_EVENTS),
        },
        CoClass {
            name: "UnixConnectionListener",
            clsid: act::CLSID_UNIX_CONNECTION_LISTENER,
            default_iface: IID_DUNIX_LISTENER,
            source_iface: Some(IID_DLISTENER_EVENTS),
        },
        CoClass {
            name: "HyperVConnectionListener",
            clsid: act::CLSID_HYPER_V_CONNECTION_LISTENER,
            default_iface: IID_DHYPERV_LISTENER,
            source_iface: Some(IID_DLISTENER_EVENTS),
        },
        CoClass {
            name: "NamedPipeClient",
            clsid: act::CLSID_NAMED_PIPE_CLIENT,
            default_iface: IID_DNAMED_PIPE_CLIENT,
            source_iface: None,
        },
        CoClass {
            name: "UnixStreamSocket",
            clsid: act::CLSID_UNIX_STREAM_SOCKET,
            default_iface: IID_DUNIX_SOCKET,
            source_iface: None,
        },
        CoClass {
            name: "HvStreamSocket",
            clsid: act::CLSID_HV_STREAM_SOCKET,
            default_iface: IID_DHV_SOCKET,
            source_iface: None,
        },
    ]
}

/// Where the generated library is kept. See the module docs for why here.
pub(crate) fn tlb_path() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        windows_core::Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED)
    })?;
    Ok(PathBuf::from(local)
        .join("rustwinui3loop")
        .join("rustwinui3loop.tlb"))
}

/// Generates the library and registers it for this user.
///
/// Idempotent, like the rest of registration: the file is rewritten and
/// re-registered, which is how a rebuilt member table is picked up.
pub(crate) fn register() -> Result<()> {
    let path = tlb_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            windows_core::Error::from_hresult(windows_core::HRESULT::from_win32(
                e.raw_os_error().unwrap_or(0) as u32,
            ))
        })?;
    }
    unsafe { build(&path)? };

    let wide = HSTRING::from(path.as_os_str());
    // Read it back rather than keeping the creation object: `RegisterTypeLib`
    // wants a loaded library, and loading the file is also the first check
    // that what was written is a library at all.
    let lib = unsafe { LoadTypeLibEx(&wide, REGKIND_NONE) }?;
    // The per-user form, to match the HKCU class registration. The
    // machine-wide `RegisterTypeLib` would need elevation.
    unsafe { RegisterTypeLibForUser(&lib, &wide, PCWSTR::null()) }
}

/// Unregisters the library and deletes the file.
///
/// A library that was never registered is not an error, for the same reason
/// a missing class key is not: `regsvr32 /u` twice should succeed twice.
pub(crate) fn unregister() -> Result<()> {
    let _ =
        unsafe { UnRegisterTypeLibForUser(&LIBID, VERSION.0, VERSION.1, LCID_NEUTRAL, SYS_WIN64) };
    // `UnRegisterTypeLibForUser` only removes the LCID it is told about, so
    // a library registered under a different one — an earlier build that
    // took the user's default locale — would survive it. The whole subtree
    // is ours, and removing it is what makes re-registration reliable.
    let _ = unsafe {
        windows::Win32::System::Registry::RegDeleteTreeW(
            windows::Win32::System::Registry::HKEY_CURRENT_USER,
            &HSTRING::from(format!(
                r"Software\Classes\TypeLib\{}",
                super::activation::clsid_key(&LIBID)
            )),
        )
    };
    if let Ok(path) = tlb_path() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// The `ITypeInfo` for one dispinterface, for `IDispatch::GetTypeInfo`.
///
/// Reads the registered library rather than the file, so a caller that
/// somehow has an object without registration gets a clean
/// `TYPE_E_LIBNOTREGISTERED` instead of type info describing a library
/// nothing else can find.
pub(crate) fn type_info(iid: &GUID) -> Result<ITypeInfo> {
    let lib: ITypeLib = unsafe { LoadRegTypeLib(&LIBID, VERSION.0, VERSION.1, 0) }?;
    unsafe { lib.GetTypeInfoOfGuid(iid) }
}

/// Writes the library to `path`.
///
/// # Safety
///
/// Drives `ICreateTypeLib2`, which is a COM API; the descriptors built below
/// borrow stack storage that must outlive each `AddFuncDesc` call, which is
/// why the parameter arrays are held in locals across it.
unsafe fn build(path: &std::path::Path) -> Result<()> {
    let lib = unsafe { CreateTypeLib2(SYS_WIN64, &HSTRING::from(path.as_os_str())) }?;
    unsafe {
        lib.SetName(&HSTRING::from(LIBRARY_NAME))?;
        lib.SetGuid(&LIBID)?;
        lib.SetVersion(VERSION.0, VERSION.1)?;
        // Locale-neutral, explicitly. `CreateTypeLib2` otherwise takes the
        // user's default, which registers the library under that LCID —
        // and then `LoadRegTypeLib` asking for the neutral 0 does not find
        // it. Every name here is English and locale-independent anyway.
        lib.SetLcid(LCID_NEUTRAL)?;
        lib.SetDocString(&HSTRING::from(
            "Named pipe, AF_UNIX and Hyper-V socket transports",
        ))?;
    }

    // `IDispatch` itself, which every dispinterface inherits. Taken from
    // stdole2 because a dispinterface's base must be the real one, not a
    // redeclaration.
    let stdole = unsafe { LoadTypeLibEx(&HSTRING::from("stdole2.tlb"), REGKIND_NONE) }?;
    let idispatch = unsafe {
        stdole.GetTypeInfoOfGuid(&windows::Win32::System::Com::IDispatch::IID)
    }?;

    // Kept so the coclass pass can point at them: `ICreateTypeLib2` has no
    // lookup of its own, and a type being defined is reached through the
    // `ICreateTypeInfo` that defines it.
    let mut defined: Vec<(GUID, ITypeInfo)> = Vec::new();

    let interfaces = dispinterfaces();
    for face in &interfaces {
        let info = unsafe { lib.CreateTypeInfo(&HSTRING::from(face.name), TKIND_DISPATCH) }?;
        unsafe {
            info.SetGuid(&face.iid)?;
            info.SetVersion(VERSION.0, VERSION.1)?;
            // `FDISPATCHABLE` alone. `FDUAL` would claim a vtable behind
            // these members, which is the one thing a dispinterface does
            // not have — `LayOut` rejects it with TYPE_E_BADMODULEKIND.
            info.SetTypeFlags(TYPEFLAG_FDISPATCHABLE.0 as u32)?;
            // `AddRefTypeInfo`'s out-parameter is typed `*const u32` in
            // these bindings, so the reference is shared even though the
            // callee writes through it.
            let href = 0u32;
            info.AddRefTypeInfo(&idispatch, &href)?;
            info.AddImplType(0, href)?;
        }
        for (index, member) in face.members.iter().enumerate() {
            unsafe { add_member(&info, index as u32, member)? };
        }
        unsafe { info.LayOut()? };
        defined.push((face.iid, info.cast()?));
    }

    for class in coclasses() {
        let info = unsafe { lib.CreateTypeInfo(&HSTRING::from(class.name), TKIND_COCLASS) }?;
        unsafe {
            info.SetGuid(&class.clsid)?;
            info.SetVersion(VERSION.0, VERSION.1)?;
            info.SetTypeFlags(TYPEFLAG_FCANCREATE.0 as u32)?;
        }
        let mut slot = 0u32;
        for (iid, source) in [(Some(class.default_iface), false), (class.source_iface, true)] {
            let Some(iid) = iid else { continue };
            let face = defined
                .iter()
                .find(|(declared, _)| *declared == iid)
                .map(|(_, info)| info.clone())
                .expect("coclass names a dispinterface this library does not declare");
            let href = 0u32;
            unsafe {
                info.AddRefTypeInfo(&face, &href)?;
                info.AddImplType(slot, href)?;
                // `[default]` on both: the first is what an early-bound
                // variable is typed as, the second is the interface a host
                // hands its event sink to.
                let flags = if source {
                    IMPLTYPEFLAG_FDEFAULT | IMPLTYPEFLAG_FSOURCE
                } else {
                    IMPLTYPEFLAG_FDEFAULT
                };
                info.SetImplTypeFlags(slot, flags)?;
            }
            slot += 1;
        }
        unsafe { info.LayOut()? };
    }

    unsafe { lib.SaveAllChanges() }
}

/// Adds one member, with its parameter and return types.
///
/// # Safety
///
/// `info` must be an open `ICreateTypeInfo` that has not been laid out.
unsafe fn add_member(info: &ICreateTypeInfo, index: u32, member: &Member) -> Result<()> {
    // Held across `AddFuncDesc`, which reads through these pointers.
    let mut params: Vec<ELEMDESC> = member
        .params
        .iter()
        .map(|p| ELEMDESC {
            tdesc: TYPEDESC {
                Anonymous: Default::default(),
                vt: p.vt.varenum(),
            },
            Anonymous: ELEMDESC_0 {
                paramdesc: PARAMDESC {
                    pparamdescex: core::ptr::null_mut(),
                    wParamFlags: PARAMFLAG_FIN,
                },
            },
        })
        .collect();

    let desc = FUNCDESC {
        memid: member.dispid,
        lprgscode: core::ptr::null_mut(),
        lprgelemdescParam: params.as_mut_ptr(),
        funckind: FUNC_DISPATCH,
        invkind: INVOKE_FUNC,
        callconv: CC_STDCALL,
        cParams: member.params.len() as i16,
        cParamsOpt: 0,
        oVft: 0,
        cScodes: 0,
        elemdescFunc: ELEMDESC {
            tdesc: TYPEDESC {
                Anonymous: Default::default(),
                vt: member.returns.varenum(),
            },
            Anonymous: ELEMDESC_0 {
                idldesc: IDLDESC::default(),
            },
        },
        wFuncFlags: windows::Win32::System::Com::FUNCFLAGS(0),
    };
    unsafe { info.AddFuncDesc(index, &desc)? };

    // Name list: the member first, then one per parameter.
    let member_name = HSTRING::from(member.name);
    let param_names: Vec<HSTRING> = member.params.iter().map(|p| HSTRING::from(p.name)).collect();
    let mut names: Vec<PCWSTR> = Vec::with_capacity(1 + param_names.len());
    names.push(PCWSTR(member_name.as_ptr()));
    names.extend(param_names.iter().map(|n| PCWSTR(n.as_ptr())));
    unsafe { info.SetFuncAndParamNames(index, &names) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::TYPEATTR;

    /// Registration is per-user and global, so two tests doing it at once
    /// would unregister each other's library mid-read. This serializes
    /// them and tears down on the way out, panic or not.
    ///
    /// The lock is poisoned-tolerant: a test that fails while holding it
    /// has already cleaned up through this guard's `Drop`, and the next
    /// test registering afresh is exactly the recovery.
    static REGISTRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Registered(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

    impl Registered {
        fn acquire() -> Self {
            let guard = REGISTRATION.lock().unwrap_or_else(|e| e.into_inner());
            // Whatever an earlier run left behind goes first: re-registering
            // over a stale library is how a changed member table is picked
            // up, and the tests must not depend on which ran before.
            unregister().ok();
            register().expect("register");
            Registered(guard)
        }
    }

    impl Drop for Registered {
        fn drop(&mut self) {
            let _ = unregister();
        }
    }

    /// The claim this module exists to make: the library describes exactly
    /// what `Invoke` implements.
    ///
    /// Walks every dispinterface the library declares and checks each
    /// member's name, DISPID and parameter count against the table it was
    /// generated from. Because both sides are read back out of the written
    /// `.tlb`, this catches a generator that silently dropped or reordered
    /// something, not just a mismatched constant.
    #[test]
    fn the_library_describes_exactly_what_invoke_implements() {
        let _registered = Registered::acquire();

        for face in dispinterfaces() {
            let info = type_info(&face.iid)
                .unwrap_or_else(|e| panic!("no type info for {}: {e}", face.name));

            let attr: *mut TYPEATTR = unsafe { info.GetTypeAttr() }.expect("GetTypeAttr");
            let count = unsafe { (*attr).cFuncs } as u32;
            let declared_iid = unsafe { (*attr).guid };
            unsafe { info.ReleaseTypeAttr(attr) };

            assert_eq!(declared_iid, face.iid, "{} has the wrong IID", face.name);
            assert_eq!(
                count as usize,
                face.members.len(),
                "{} declares {} members, the table has {}",
                face.name,
                count,
                face.members.len()
            );

            for (index, member) in face.members.iter().enumerate() {
                let desc = unsafe { info.GetFuncDesc(index as u32) }.expect("GetFuncDesc");
                let (memid, params) = unsafe { ((*desc).memid, (*desc).cParams) };
                unsafe { info.ReleaseFuncDesc(desc) };

                assert_eq!(
                    memid, member.dispid,
                    "{}::{} has DISPID {memid}, the table says {}",
                    face.name, member.name, member.dispid
                );
                assert_eq!(
                    params as usize,
                    member.params.len(),
                    "{}::{} takes {params} parameters, the table says {}",
                    face.name,
                    member.name,
                    member.params.len()
                );

                // Names come back through a separate call, which is where a
                // generator that paired a FUNCDESC with the wrong name list
                // would show up.
                let mut names = [windows_core::BSTR::new()];
                let mut got = 0u32;
                unsafe { info.GetNames(memid, &mut names, &mut got) }.expect("GetNames");
                assert_eq!(names[0].to_string(), member.name, "{} member name", face.name);
            }
        }
    }

    /// The library and the activation table have to agree about which
    /// classes exist, or a host would early-bind to a coclass that cannot
    /// be created — or miss one that can.
    #[test]
    fn every_activatable_class_is_in_the_library() {
        let declared: Vec<GUID> = coclasses().iter().map(|c| c.clsid).collect();
        let activatable: Vec<GUID> = super::super::activation::CLASSES
            .iter()
            .map(|c| c.clsid)
            .collect();
        assert_eq!(declared.len(), activatable.len());
        for clsid in activatable {
            assert!(
                declared.contains(&clsid),
                "{} is activatable but absent from the library",
                super::super::activation::clsid_key(&clsid)
            );
        }
    }

    /// Only the listeners raise events, and they all raise the same one.
    #[test]
    fn only_listeners_declare_a_source_interface() {
        for class in coclasses() {
            let listener = class.name.ends_with("ConnectionListener");
            assert_eq!(
                class.source_iface.is_some(),
                listener,
                "{} source interface",
                class.name
            );
            if listener {
                assert_eq!(class.source_iface, Some(IID_DLISTENER_EVENTS));
            }
        }
    }

    /// What a host reads to find the event: the coclass's implemented
    /// types, and the flags saying which is the default and which is the
    /// source. `IProvideClassInfo::GetClassInfo` hands this exact type info
    /// out, so if it is wrong here no host can discover the event.
    #[test]
    fn listener_coclasses_declare_the_source_interface() {
        let _registered = Registered::acquire();

        for class in coclasses() {
            let info = type_info(&class.clsid)
                .unwrap_or_else(|e| panic!("no type info for {}: {e}", class.name));
            let attr: *mut TYPEATTR = unsafe { info.GetTypeAttr() }.expect("GetTypeAttr");
            let impls = unsafe { (*attr).cImplTypes } as u32;
            unsafe { info.ReleaseTypeAttr(attr) };

            let expected = if class.source_iface.is_some() { 2 } else { 1 };
            assert_eq!(impls, expected, "{} implemented types", class.name);

            for index in 0..impls {
                let href = unsafe { info.GetRefTypeOfImplType(index) }.expect("GetRefTypeOfImplType");
                let face = unsafe { info.GetRefTypeInfo(href) }.expect("GetRefTypeInfo");
                let fattr: *mut TYPEATTR = unsafe { face.GetTypeAttr() }.expect("GetTypeAttr");
                let iid = unsafe { (*fattr).guid };
                unsafe { face.ReleaseTypeAttr(fattr) };
                let flags = unsafe { info.GetImplTypeFlags(index) }.expect("GetImplTypeFlags");

                if index == 0 {
                    assert_eq!(iid, class.default_iface, "{} default interface", class.name);
                    assert!(
                        flags.0 & IMPLTYPEFLAG_FDEFAULT.0 != 0,
                        "{} default interface is not marked default",
                        class.name
                    );
                    assert!(
                        flags.0 & IMPLTYPEFLAG_FSOURCE.0 == 0,
                        "{} default interface must not be a source",
                        class.name
                    );
                } else {
                    assert_eq!(Some(iid), class.source_iface, "{} source", class.name);
                    assert!(
                        flags.0 & IMPLTYPEFLAG_FSOURCE.0 != 0,
                        "{} source is not marked FSOURCE",
                        class.name
                    );
                    assert!(
                        flags.0 & IMPLTYPEFLAG_FDEFAULT.0 != 0,
                        "{} source is not the default source",
                        class.name
                    );
                }
            }
        }
    }
}
