//! The activation path: how a caller outside this process makes the first
//! object.
//!
//! Everything else in `comio` builds objects that a *Rust* caller constructs
//! directly — `NamedPipeConnectionListener::new()` hands back a `ComObject`,
//! and the caller takes it from there by `QueryInterface`. That covers
//! in-process use and nothing else: a C++, C# or Python host has no way to
//! make the first object, because nothing tells the runtime that these
//! classes exist or which module holds them.
//!
//! This module is that missing half. It gives each of the six public classes
//! a CLSID and a runtime class name and exports the two entry points an
//! in-process server is found through.
//!
//! # Two activation models, one table
//!
//! `DllGetActivationFactory` hands the Windows Runtime an
//! `IActivationFactory` for a runtime class name; `DllGetClassObject` hands
//! COM an `IClassFactory` for a CLSID. Both read the same [`CLASSES`] table,
//! whose names come from the generated class types in [`super::bindings`].
//! One object, one name, two front doors.
//!
//! WinRT is the front door that matters, and it is the one the published
//! `.winmd` describes. COM is kept because a WinRT interface *is* a COM
//! interface: a caller who has registered the CLSID themselves can
//! `CoCreateInstance` and get a working object with the same vtables, without
//! an activation context. That is the only thing the COM half is for.
//!
//! # This module does not write the registry
//!
//! It used to, through `DllRegisterServer` / `DllUnregisterServer`. Those
//! exports are gone as of 2026-11-09, along with every registry-writing
//! function behind them, because the half that mattered could never work and
//! the half that worked had no consumer.
//!
//! WinRT **cannot** be reached by registration from a component like this
//! one. The documented machine-wide path is
//! `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`, and that tree
//! is owned by `NT SERVICE\TrustedInstaller`: Administrators hold *read*
//! access only, so an elevated `regsvr32` was denied exactly like an ordinary
//! one. Measured on Windows 11 26100, where every one of the 4,819 registered
//! classes resolves to a DLL inside `C:\Windows` and not one is third-party.
//! The per-user branch is no help either: the same keys under
//! `HKEY_CURRENT_USER` leave `RoActivateInstance` answering
//! `REGDB_E_CLASSNOTREG`, because that branch is read for packaged apps.
//!
//! So `DllRegisterServer` reported success while delivering only its COM
//! half — and that half wrote `HKCU` CLSID keys for a path with no
//! demonstrated consumer, since both shipped projections (`python/`,
//! `csharp/`) are reg-free. It was the same shape of untruth as the ProgIDs
//! removed before it, one level up, so it went the same way. Self-registration
//! is also the mechanism Microsoft has been steering installers away from for
//! a decade; MSIX and reg-free activation are the replacements.
//!
//! What is left is a component that **describes** itself and never mutates
//! the machine. [`manifest`] generates the reg-free activation manifest, and
//! [`ComClass`] carries everything an installer would need — CLSID, class
//! name, description — for the caller to write if they want the COM door
//! open. The integration test does exactly that, which is how the COM path
//! stays proven without this module claiming to manage it.
//!
//! # How WinRT is actually reached
//!
//! Reg-free, through [`manifest`]: the consuming process supplies it as its
//! own application manifest, or loads it at run time with `CreateActCtx` +
//! `ActivateActCtx`. The second form is what a shared interpreter like
//! `python.exe` needs, since its manifest is not ours to edit. Either way the
//! runtime calls `DllGetActivationFactory` here, against the same metadata.
//!
//! `threadingModel="both"` says the objects are safe to use from an STA or
//! the MTA. They are: every one of them is `Send + Sync` and already delivers
//! completions on thread-pool threads. It also keeps the in-apartment case
//! the normal one — the object is created directly in the caller's apartment
//! rather than in a host STA it would then have to marshal out of.
//!
//! Marshalling *between* apartments is a separate question, and the answer is
//! the `.winmd`: the metadata is what gives the runtime the interface
//! descriptions it needs. (Until 2026-11-09 these classes marshalled through
//! oleaut32's dispatch proxy, because they implemented `IDispatch` and shipped
//! a type library. That layer is deleted — it could only describe a *blocking
//! translation* of this surface, since automation has no notion of a
//! parameterized interface and the vtable's `IAsyncOperation` returns could
//! not appear in it at all. Scripts come back through a projection, with the
//! async model intact.)
//!
//! # Why there is no `DllCanUnloadNow`
//!
//! There is nothing honest to put in it. Returning `S_OK` requires knowing
//! that no object this module created is still alive, and nothing here can
//! know that — `#[implement]` gives no hook on the construction and
//! destruction of every object, the same types are also constructed from pure
//! Rust paths that have no reason to touch a module count, and a `ComObject`
//! handed to a caller can outlive any count we did keep. Unloading a module
//! that still owns live vtables, in-flight overlapped operations and
//! thread-pool callbacks is a crash.
//!
//! So the only sound answer was an unconditional `S_FALSE`, and *not exporting
//! the function says the same thing*: `CoFreeUnusedLibraries` and
//! `RoFreeUnusedLibraries` resolve it by `GetProcAddress` and leave a module
//! that does not export it loaded. The export was ceremony around a refusal
//! the absence already expresses. The DLL stays resident for the life of the
//! process either way.

use super::unload::ModuleRef;
use core::ffi::c_void;
use super::bindings::Pbk20191::WinAsync::Comio as rc;
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, S_OK,
};
// Only `DllCanUnloadNow` needs it, and only a `dylib` build has one.
#[cfg(feature = "dylib")]
use windows::Win32::Foundation::S_FALSE;
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::Com::IClassFactory_Impl;
use windows::Win32::System::WinRT::{IActivationFactory, IActivationFactory_Impl};
use windows_core::{
    BOOL, GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface, Ref, Result, implement,
};

/// One activatable class: the CLSID a caller passes to `CoCreateInstance`,
/// the runtime class name `RoActivateInstance` resolves, and the
/// constructor behind both.
///
/// The constructor returns `IUnknown` rather than the concrete type because
/// that is all activation needs — the caller asked for an IID, and the
/// factory answers it by `QueryInterface`, exactly as it would for an object
/// it had been handed from anywhere else.
pub struct ComClass {
    /// Frozen. Changing one breaks every caller that already compiled it in.
    pub clsid: GUID,
    /// The runtime class name: what `RoActivateInstance` is given, what
    /// `GetRuntimeClassName` answers, and the key `DllGetActivationFactory`
    /// looks up.
    ///
    /// Always taken from [`super::bindings`] via `class_name`, never
    /// written out here, so the name in the metadata and the name in this
    /// table cannot drift apart.
    ///
    /// This used to be registered as a ProgID as well. It is not any more. A
    /// ProgID exists so a *script* can say
    /// `New-Object -ComObject Pbk20191.WinAsync.Comio.NamedPipeConnector`, and since
    /// the `IDispatch` layer went (2026-11-09) that would hand a script an
    /// object it has successfully created and cannot call a single method on.
    /// Advertising the name was worse than not advertising it. A compiled
    /// caller uses the CLSID, and a WinRT caller uses this name through
    /// `DllGetActivationFactory`.
    pub name: &'static str,
    /// Human-readable. Nothing here consumes it: it is carried for whoever
    /// writes the CLSID key, where it is the conventional default value.
    pub description: &'static str,
    create: fn() -> IUnknown,
}

/// Every class this module can activate.
///
/// The three listeners and the three connectors — the same six types the
/// Rust API exposes, under the same split: one concrete type per transport,
/// so a caller that holds one can only ask for the addressing that transport
/// actually has.
pub static CLASSES: &[ComClass] = &[
    ComClass {
        clsid: CLSID_NAMED_PIPE_CONNECTION_LISTENER,
        name: class_name::<rc::NamedPipeConnectionListener>(),
        description: "Named pipe connection listener",
        create: || super::NamedPipeConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_UNIX_CONNECTION_LISTENER,
        name: class_name::<rc::UnixConnectionListener>(),
        description: "AF_UNIX connection listener",
        create: || super::UnixConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_HYPER_V_CONNECTION_LISTENER,
        name: class_name::<rc::HyperVConnectionListener>(),
        description: "Hyper-V socket connection listener",
        create: || super::HyperVConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_NAMED_PIPE_CONNECTOR,
        name: class_name::<rc::NamedPipeConnector>(),
        description: "Named pipe client connector",
        create: || super::NamedPipeConnector::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_UNIX_CONNECTOR,
        name: class_name::<rc::UnixConnector>(),
        description: "AF_UNIX stream connector",
        create: || super::UnixConnector::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_HYPER_V_CONNECTOR,
        name: class_name::<rc::HyperVConnector>(),
        description: "Hyper-V socket stream connector",
        create: || super::HyperVConnector::new().into_interface(),
    },
];

// The six CLSIDs. Frozen — do not change or reuse, for the same reason the
// interface GUIDs are frozen: a caller compiles these in.
pub const CLSID_NAMED_PIPE_CONNECTION_LISTENER: GUID =
    GUID::from_u128(0x5535a7e3_d62f_4a8c_8244_67b6bf1bb809);
pub const CLSID_UNIX_CONNECTION_LISTENER: GUID =
    GUID::from_u128(0x60be1b42_01c6_486d_9b68_4e7af1503e30);
pub const CLSID_HYPER_V_CONNECTION_LISTENER: GUID =
    GUID::from_u128(0xbf4265d3_3218_4f06_ad62_00d9edc2ffd3);
pub const CLSID_NAMED_PIPE_CONNECTOR: GUID = GUID::from_u128(0x2ad42747_717e_4994_9457_5e2b1ba48247);
pub const CLSID_UNIX_CONNECTOR: GUID = GUID::from_u128(0xdb436265_333f_4f3e_a7fd_a3f842a639d4);
pub const CLSID_HYPER_V_CONNECTOR: GUID = GUID::from_u128(0xbe6a6ff1_e2b8_4014_b608_168668baf43d);

/// The runtime class name `C` answers `GetRuntimeClassName` with.
///
/// A `const fn` so [`CLASSES`] can be a `static`: `RuntimeName` is the trait
/// `windows-rs` reads that name through, and naming it here rather than
/// repeating the literal is what makes this table's name and the name the
/// object reports the same string by construction.
const fn class_name<C: windows_core::RuntimeName>() -> &'static str {
    C::NAME
}

/// Looks a CLSID up in [`CLASSES`].
pub fn class_for(clsid: &GUID) -> Option<&'static ComClass> {
    CLASSES.iter().find(|c| &c.clsid == clsid)
}

/// The class object COM asks for before it asks for an instance.
///
/// One factory per class rather than one switching on a CLSID: COM hands the
/// factory out and may hold it across many `CreateInstance` calls, so the
/// class it makes is settled once, here, instead of being re-decided on
/// every call.
#[implement(IClassFactory)]
struct ClassFactory(&'static ComClass, ModuleRef);

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() {
            return Err(windows_core::Error::from_hresult(
                windows::Win32::Foundation::E_POINTER,
            ));
        }
        // COM requires the out-parameter be cleared before anything else can
        // fail, so a caller that ignores the HRESULT still sees null rather
        // than an uninitialised pointer it would go on to release.
        unsafe { *object = core::ptr::null_mut() };
        if iid.is_null() {
            return Err(windows_core::Error::from_hresult(
                windows::Win32::Foundation::E_POINTER,
            ));
        }
        // Aggregation would require these objects to delegate IUnknown to a
        // controlling outer object. They do not, and `#[implement]` has no
        // way to make them; saying so is the contract.
        if !outer.is_null() {
            return Err(windows_core::Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        let unknown = (self.0.create)();
        unsafe { unknown.query(iid, object).ok() }
    }

    /// Honoured: a lock is a caller saying "keep this module loaded even
    /// with no objects out", and `DllCanUnloadNow` now has an answer that
    /// can change, so the claim has somewhere to land.
    ///
    /// It used to be accepted and ignored, which was sound only because the
    /// answer was an unconditional refusal.
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        super::unload::lock_server(lock.as_bool());
        Ok(())
    }
}

/// Hands COM the class object for `clsid`.
///
/// The first of the four exports that make this module an in-process COM
/// server: `CoCreateInstance` reads `InprocServer32` from the registry,
/// loads the module it names, and calls this.
///
/// # Safety
///
/// Called by the COM runtime with a valid CLSID, IID and out-pointer.
pub unsafe fn dll_get_class_object(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if object.is_null() {
        return windows::Win32::Foundation::E_POINTER;
    }
    unsafe { *object = core::ptr::null_mut() };
    if clsid.is_null() || iid.is_null() {
        return windows::Win32::Foundation::E_POINTER;
    }
    let Some(class) = class_for(unsafe { &*clsid }) else {
        return CLASS_E_CLASSNOTAVAILABLE;
    };
    let factory: IClassFactory = ClassFactory(class, ModuleRef::new()).into();
    unsafe { factory.query(iid, object) }
}

/// REGDB_E_CLASSNOTREG — the documented answer when this module does not
/// implement the requested runtime class.
const REGDB_E_CLASSNOTREG: HRESULT = HRESULT(0x80040154u32 as i32);

/// Looks a runtime class name up in [`CLASSES`].
///
/// Matched against [`ComClass::name`], which is the same constant the object
/// answers `GetRuntimeClassName` with — so a class renamed in the metadata
/// cannot keep activating under its old name here.
pub fn class_for_name(name: &str) -> Option<&'static ComClass> {
    CLASSES.iter().find(|c| c.name == name)
}

/// The WinRT counterpart of [`ClassFactory`]: what `RoActivateInstance` asks
/// for before it asks for an instance.
///
/// Simpler than the COM one because WinRT split the two concerns the class
/// factory conflates. `IActivationFactory::ActivateInstance` takes no IID and
/// no aggregation outer — it returns the object as `IInspectable` and the
/// caller queries from there — so the aggregation refusal and the
/// out-parameter clearing that `IClassFactory::CreateInstance` needs have no
/// counterpart here.
///
/// Only the six classes marked `#[Activatable]` in the metadata reach this.
/// `TransportStream` and `ConnectionReceivedArgs` are runtime classes too,
/// but they are only ever produced by a connect or an accept, and the
/// metadata says so — there is nothing to keep in step by hand.
#[implement(IActivationFactory)]
struct ActivationFactory(&'static ComClass, ModuleRef);

impl IActivationFactory_Impl for ActivationFactory_Impl {
    fn ActivateInstance(&self) -> Result<IInspectable> {
        // The same constructor `CoCreateInstance` reaches, queried to the
        // interface WinRT hands back rather than the one COM does. Both
        // succeed on every class here: these objects are `IInspectable`
        // because every interface they implement now derives from it.
        (self.0.create)().cast()
    }
}

/// Hands the Windows Runtime the activation factory for `name`.
///
/// The WinRT sibling of `DllGetClassObject`, and the export that makes the
/// published `.winmd` mean something: a projection reads the metadata, calls
/// `RoActivateInstance`, and the runtime loads this module and calls here.
/// Without it the metadata describes classes nothing can create.
///
/// Return values follow the documented contract: `E_INVALIDARG` for a null
/// argument, `REGDB_E_CLASSNOTREG` for a class this module does not
/// implement.
///
/// # Safety
///
/// Called by the Windows Runtime with a valid `HSTRING` and out-pointer. The
/// string stays the caller's: it arrives by value as a borrowed handle, which
/// is why it is taken as a `ManuallyDrop` rather than an owned `HSTRING` that
/// would be released on return.
pub unsafe fn dll_get_activation_factory(
    name: core::mem::ManuallyDrop<HSTRING>,
    factory: *mut *mut c_void,
) -> HRESULT {
    if factory.is_null() {
        return E_INVALIDARG;
    }
    // Cleared first, for the same reason `DllGetClassObject` clears its own:
    // a caller that ignores the HRESULT sees null rather than a pointer it
    // would go on to release.
    unsafe { *factory = core::ptr::null_mut() };
    // A null `HSTRING` handle *is* the empty string, so this covers both the
    // documented null case and a caller asking for a class called "".
    if name.is_empty() {
        return E_INVALIDARG;
    }
    let Some(class) = class_for_name(&name.to_string()) else {
        return REGDB_E_CLASSNOTREG;
    };
    let activation: IActivationFactory = ActivationFactory(class, ModuleRef::new()).into();
    unsafe { *factory = activation.into_raw() };
    S_OK
}

/// Whether this module may be unloaded.
///
/// `S_OK` only when nothing this module owns can still execute: no live
/// objects, no server locks, no overlapped operations in flight. See
/// [`super::unload`] for what is counted and why that list is the right one.
///
/// Unloading a module that still holds a vtable, a pending completion or a
/// queued thread-pool callback is a jump into freed code, so the two possible
/// mistakes are not symmetric. A wrong `S_FALSE` costs a resident DLL until
/// the next sweep; a wrong `S_OK` is a crash. Everything here is arranged so
/// that only the first one is reachable.
/// Present only in a `dylib` build, because only that build keeps the counts
/// the answer comes from. Without the export COM never asks, which is the
/// same outcome as an unconditional `S_FALSE` — see [`super::unload`].
#[cfg(feature = "dylib")]
pub fn dll_can_unload_now() -> HRESULT {
    if !super::unload::can_unload() {
        return S_FALSE;
    }
    // Saying yes is also the moment to give back what unloading would
    // otherwise strand -- `WSAStartup`'s process reference. See
    // `unload::release_process_state` for why here and nowhere else.
    super::unload::release_process_state();
    S_OK
}

/// The reg-free activation manifest for this component.
///
/// This, not the registry, is how a non-packaged third-party component is
/// reached — and since this module stopped writing the registry it is the
/// only thing here that makes these classes activatable. The documented
/// machine-wide alternative cannot be written at all: the
/// `ActivatableClassId` tree is owned by `NT SERVICE{B}TrustedInstaller` and
/// Administrators hold *read* access only, so even an elevated writer is
/// denied. Measured on Windows 11 26100, where all 4,819 registered classes
/// resolve to a DLL inside `C:{B}Windows` and not one is third-party.
///
/// A consuming process supplies this manifest instead, either as its own
/// application manifest or by loading it at run time with `CreateActCtx` +
/// `ActivateActCtx`. The second form is what a shared interpreter like
/// `python.exe` needs, since its manifest is not ours to edit. Nothing else
/// changes: the runtime still calls `DllGetActivationFactory` in this
/// module, and the metadata is the same metadata.
///
/// `<file name>` carries no path. It resolves against the directory holding
/// the manifest, or against `lpAssemblyDirectory` when the caller passes one
/// — which is how the manifest can live apart from the DLL it names.
///
/// Generated from [`CLASSES`] rather than written out, for the same reason
/// the class names themselves are: one list, so a class cannot appear in the
/// metadata and go missing from the manifest.
pub fn manifest() -> String {
    let mut s = String::with_capacity(256 + CLASSES.len() * 160);
    s.push_str(concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\n",
        "<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\n",
        "  <assemblyIdentity type=\"win32\" name=\"Pbk20191.WinAsync.Comio\" version=\"1.0.0.0\"/>\n",
        "  <file name=\"winasync.dll\">\n",
    ));
    for class in CLASSES {
        // `threadingModel="both"` is the manifest spelling of the claim
        // `ThreadingModel = Both` makes for COM: these objects are agile.
        s.push_str("    <activatableClass name=\"");
        s.push_str(class.name);
        s.push_str(concat!(
            "\" threadingModel=\"both\"",
            " xmlns=\"urn:schemas-microsoft-com:winrt.v1\"/>\n",
        ));
    }
    s.push_str("  </file>\n</assembly>\n");
    s
}

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` — the braced form the registry
/// uses and `CLSIDFromString` parses.
///
/// Public for the same reason [`ComClass::description`] is: this module no
/// longer writes the CLSID keys, so whoever does needs the spelling. The
/// integration test is the first such caller.
pub fn clsid_key(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Activation end to end inside one process, minus the loader: ask for
    /// each class object the way COM does, make an instance, and check it is
    /// the class that was asked for.
    ///
    /// This is the part of the path that has no registry in it. The loader's
    /// half — registry lookup, `LoadLibrary`, `GetProcAddress` — is covered
    /// by `tests/activation.rs`, which can only run against the real DLL.
    #[test]
    fn every_class_activates_through_its_class_object() {
        // Each class is identified by an interface only it implements, so a
        // factory wired to the wrong constructor cannot pass.
        type Identified = (GUID, fn(&IUnknown) -> bool);
        let expected: &[Identified] = &[
            (CLSID_NAMED_PIPE_CONNECTION_LISTENER, |u| {
                u.cast::<super::super::INamedPipeBind>().is_ok()
            }),
            (CLSID_UNIX_CONNECTION_LISTENER, |u| {
                u.cast::<super::super::IUnixBind>().is_ok()
            }),
            (CLSID_HYPER_V_CONNECTION_LISTENER, |u| {
                u.cast::<super::super::IHyperVBind>().is_ok()
            }),
            (CLSID_NAMED_PIPE_CONNECTOR, |u| {
                u.cast::<super::super::INamedPipeConnector>().is_ok()
            }),
            (CLSID_UNIX_CONNECTOR, |u| {
                u.cast::<super::super::IUnixConnector>().is_ok()
            }),
            (CLSID_HYPER_V_CONNECTOR, |u| {
                u.cast::<super::super::IHyperVConnector>().is_ok()
            }),
        ];
        assert_eq!(expected.len(), CLASSES.len(), "a class has no coverage here");

        for (clsid, is_right_class) in expected {
            let factory: IClassFactory = unsafe {
                let mut raw = core::ptr::null_mut();
                dll_get_class_object(clsid, &IClassFactory::IID, &mut raw)
                    .ok()
                    .expect("DllGetClassObject");
                IClassFactory::from_raw(raw)
            };

            let instance: IUnknown = unsafe { factory.CreateInstance(None) }.expect("CreateInstance");
            assert!(
                is_right_class(&instance),
                "{} activated the wrong class",
                clsid_key(clsid)
            );

            // A second instance is a second object, not the same one again.
            let other: IUnknown = unsafe { factory.CreateInstance(None) }.expect("CreateInstance");
            assert_ne!(instance.as_raw(), other.as_raw());
        }
    }

    /// The refusals a class factory owes its caller.
    #[test]
    fn activation_refuses_what_it_cannot_do() {
        let mut raw = core::ptr::null_mut();

        // A CLSID this module does not serve. COM relies on this exact code
        // to go on looking elsewhere rather than failing the activation.
        let stranger = GUID::from_u128(0x00000000_0000_0000_0000_0000000000ff);
        assert_eq!(
            unsafe { dll_get_class_object(&stranger, &IClassFactory::IID, &mut raw) },
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert!(raw.is_null());

        // An interface the class object does not have.
        assert!(
            unsafe {
                dll_get_class_object(
                    &CLSID_NAMED_PIPE_CONNECTOR,
                    &super::super::IUnixConnector::IID,
                    &mut raw,
                )
            }
            .is_err()
        );
        assert!(raw.is_null());

        // Aggregation, which these objects cannot support.
        let factory: IClassFactory = unsafe {
            let mut f = core::ptr::null_mut();
            dll_get_class_object(&CLSID_UNIX_CONNECTOR, &IClassFactory::IID, &mut f)
                .ok()
                .unwrap();
            IClassFactory::from_raw(f)
        };
        let outer: IUnknown = super::super::NamedPipeConnector::new().into_interface();
        let err = unsafe { factory.CreateInstance::<_, IUnknown>(&outer) }
            .expect_err("aggregation must be refused");
        assert_eq!(err.code(), CLASS_E_NOAGGREGATION);

        // Locking is accepted even though it does nothing.
        unsafe { factory.LockServer(true) }.expect("LockServer");
        unsafe { factory.LockServer(false) }.expect("LockServer");
    }
}
