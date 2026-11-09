//! The activation path: reaching these transports by `CoCreateInstance`.
//!
//! Everything else in `comio` builds COM objects that a *Rust* caller
//! constructs directly — `NamedPipeConnectionListener::new()` hands back a
//! `ComObject`, and a COM caller takes it from there by `QueryInterface`.
//! That covers in-process use and nothing else: a C++, C# or scripting host
//! has no way to make the first object, because nothing tells the COM
//! runtime that these classes exist or which module holds them.
//!
//! This module is that missing half. It gives each of the six public classes
//! a CLSID and a ProgID, exports the entry points an in-process server
//! is defined by, and writes the registry keys that point a CLSID back at
//! this module. After `regsvr32 rustwinui3loop.dll`, any language that can
//! call `CoCreateInstance` can open a named pipe, an AF_UNIX socket or a
//! Hyper-V socket through the same WinRT stream interfaces.
//!
//! # Two activation models, one table
//!
//! This module now serves both. [`DllGetClassObject`] hands COM an
//! `IClassFactory` for a CLSID; [`DllGetActivationFactory`] hands the Windows
//! Runtime an `IActivationFactory` for a runtime class name. Both read the
//! same [`CLASSES`] table, whose names come from the generated class types in
//! [`super::bindings`]. One object, one name, two front doors.
//!
//! What is still missing is the WinRT counterpart of the registry keys below.
//! `RoActivateInstance` finds this module through
//! `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`, and that is
//! machine-wide: third-party WinRT activation outside a package is either
//! `HKEY_LOCAL_MACHINE` or a manifest on the *consuming* executable, which is
//! not an option for a shared interpreter like `python.exe`. So the
//! elevation-free `regsvr32` below does not carry over to the WinRT half.
//! `docs/comio-com-winrt-status.md` has the reasoning.
//!
//! Until those keys exist, `DllGetActivationFactory` is reachable by calling
//! the export directly — which is what the integration test does, and what
//! isolates "the factory works" from "the registry points at us".
//!
//! # What the registration does and does not claim
//!
//! `DllRegisterServer` writes under **`HKEY_CURRENT_USER\Software\Classes`**,
//! not `HKEY_CLASSES_ROOT`. The conventional choice is the latter, which is
//! `HKEY_LOCAL_MACHINE\Software\Classes` for writes and needs an elevated
//! `regsvr32`. Per-user registration needs no elevation, is what a
//! development loop actually wants, and is read by the same `CoCreateInstance`
//! lookup — `HKEY_CLASSES_ROOT` is a merged view with the per-user hive
//! taking priority. The cost is that the classes are visible to this user
//! only; a machine-wide install would point this same code at
//! `HKEY_LOCAL_MACHINE` instead.
//!
//! `ThreadingModel = Both` says the objects are safe to use from an STA or
//! the MTA. They are: every one of them is `Send + Sync` and already
//! delivers completions on thread-pool threads.
//!
//! Whether a given *interface* on them can leave the apartment it was created
//! in is a separate question, and today the answer is no. Every interface here
//! is one this crate declares itself — `IConnectionListener`, the
//! `*ListenerBind` pair, `IStreamConnector` — with no registered proxy/stub
//! and no metadata describing it, so COM has nothing to marshal them with.
//! Use them inside one apartment. (Until 2026-11-09 the dispinterfaces did
//! marshal, through oleaut32's dispatch proxy; that layer is gone. Publishing
//! a `.winmd` is what restores marshalling, this time for the real
//! interfaces.)
//!
//! `Both` is what keeps the in-apartment case the normal one: the object is
//! created directly in the caller's apartment rather than in a host STA it
//! would then have to marshal out of.
//!
//! # What a caller can do with an activated object
//!
//! Everything a vtable gives it, which is every language that can declare
//! one: C++, C# with `[ComImport]`, Rust, anything with a `DllGetClassObject`
//! of its own. `regsvr32 rustwinui3loop.dll` then `CoCreateInstance` is the
//! whole setup.
//!
//! Scripting hosts do **not** work, and that is deliberate as of 2026-11-09.
//! These classes used to implement `IDispatch` and ship a type library, so
//! `New-Object -ComObject Rustwinui3loop.NamedPipeConnector` could be driven from
//! PowerShell, VBScript and VBA. That layer is deleted. It could only describe
//! a *blocking translation* of this surface — automation has no notion of a
//! parameterized interface, so the vtable's `IAsyncOperation` returns could
//! not appear in it at all — and the whole point of the WinRT direction is to
//! stop translating. Scripts come back through a `.winmd` and a projection,
//! with the async model intact.
//!
//! Registrations written by a build from before that change leave stale
//! `Interface\{…}` and `TypeLib` keys behind, because the IIDs they named no
//! longer exist here to delete. They are inert; `regsvr32 /u` with the old
//! binary, or ignoring them, are both fine.
//!
//! # Why `DllCanUnloadNow` always refuses
//!
//! It returns `S_FALSE` unconditionally: this module never lets itself be
//! unloaded. Returning `S_OK` requires knowing that no object this module
//! created is still alive, and nothing here can know that — `#[implement]`
//! gives no hook on the construction and destruction of every object, the
//! same types are also constructed from pure Rust paths that have no reason
//! to touch a module count, and a `ComObject` handed to a caller can outlive
//! any count we did keep. Unloading a module that still owns live vtables,
//! in-flight overlapped operations and thread-pool callbacks is a crash, so
//! the conservative answer is the only sound one. The cost is bounded and
//! well understood: the DLL stays resident for the life of the process,
//! which is what `CoFreeUnusedLibraries` would otherwise reclaim.

use core::ffi::c_void;
use super::bindings::Rustwinui3loop as rc;
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_INVALIDARG, HMODULE, S_FALSE,
    S_OK,
};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::Com::IClassFactory_Impl;
use windows::Win32::System::WinRT::{IActivationFactory, IActivationFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows_core::{
    BOOL, GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface, PCWSTR, Ref, Result,
    implement,
};

/// The registry root the shipped entry points use, split out so the
/// round-trip test can exercise the same code against a scratch location
/// instead of the live class registrations.
const CLASSES_ROOT: &str = "Software\\Classes";

/// One activatable class: the CLSID a caller passes to `CoCreateInstance`,
/// the ProgID that resolves to it, and the constructor behind both.
///
/// The constructor returns `IUnknown` rather than the concrete type because
/// that is all activation needs — the caller asked for an IID, and the
/// factory answers it by `QueryInterface`, exactly as it would for an object
/// it had been handed from anywhere else.
pub struct ComClass {
    /// Frozen. Changing one breaks every caller that already compiled it in.
    pub clsid: GUID,
    /// `Vendor.Component`, the name `CLSIDFromProgID` resolves — and, because
    /// both are taken from [`super::bindings`], the same string the object
    /// answers `GetRuntimeClassName` with and the key
    /// [`DllGetActivationFactory`] looks up.
    pub progid: &'static str,
    /// Human-readable; becomes the default value of the CLSID key.
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
        progid: class_name::<rc::NamedPipeConnectionListener>(),
        description: "Named pipe connection listener",
        create: || super::NamedPipeConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_UNIX_CONNECTION_LISTENER,
        progid: class_name::<rc::UnixConnectionListener>(),
        description: "AF_UNIX connection listener",
        create: || super::UnixConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_HYPER_V_CONNECTION_LISTENER,
        progid: class_name::<rc::HyperVConnectionListener>(),
        description: "Hyper-V socket connection listener",
        create: || super::HyperVConnectionListener::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_NAMED_PIPE_CONNECTOR,
        progid: class_name::<rc::NamedPipeConnector>(),
        description: "Named pipe client connector",
        create: || super::NamedPipeConnector::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_UNIX_CONNECTOR,
        progid: class_name::<rc::UnixConnector>(),
        description: "AF_UNIX stream connector",
        create: || super::UnixConnector::new().into_interface(),
    },
    ComClass {
        clsid: CLSID_HYPER_V_CONNECTOR,
        progid: class_name::<rc::HyperVConnector>(),
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
/// repeating the literal is what makes the ProgID and the runtime class name
/// the same string by construction.
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
struct ClassFactory(&'static ComClass);

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

    /// Accepted and ignored, because [`DllCanUnloadNow`] never agrees to
    /// unload in the first place. Failing it instead would break callers
    /// that lock the server as a matter of course.
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
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
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
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
    let factory: IClassFactory = ClassFactory(class).into();
    unsafe { factory.query(iid, object) }
}

/// REGDB_E_CLASSNOTREG — the documented answer when this module does not
/// implement the requested runtime class.
const REGDB_E_CLASSNOTREG: HRESULT = HRESULT(0x80040154u32 as i32);

/// Looks a runtime class name up in [`CLASSES`].
///
/// The name is the same string as the ProgID — both are taken from
/// [`super::bindings`] — so one table answers both activation models.
pub fn class_for_name(name: &str) -> Option<&'static ComClass> {
    CLASSES.iter().find(|c| c.progid == name)
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
struct ActivationFactory(&'static ComClass);

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
/// The WinRT sibling of [`DllGetClassObject`], and the export that makes the
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
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetActivationFactory(
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
    let activation: IActivationFactory = ActivationFactory(class).into();
    unsafe { *factory = activation.into_raw() };
    S_OK
}

/// Refuses to be unloaded; see the module docs for why this is the sound
/// answer rather than a lazy one.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

/// Writes the registry keys that let `CoCreateInstance` find this module.
///
/// This is what `regsvr32` calls. It is idempotent: re-registering an
/// already-registered module just rewrites the same values, which is how a
/// rebuild at a new path is picked up.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match unsafe { register_under(HKEY_CURRENT_USER, CLASSES_ROOT) } {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

/// Removes everything [`DllRegisterServer`] wrote. What `regsvr32 /u` calls.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match unsafe { unregister_under(HKEY_CURRENT_USER, CLASSES_ROOT) } {
        Ok(()) => S_OK,
        Err(e) => e.code(),
    }
}

/// The body of [`DllRegisterServer`], with the destination as a parameter.
///
/// `base` is the path under `root` that holds the class registrations —
/// `Software\Classes` in the shipped entry point, a scratch key in the test.
///
/// # Safety
///
/// `root` must be a live registry key the caller may write to.
pub(crate) unsafe fn register_under(root: HKEY, base: &str) -> Result<()> {
    let module = unsafe { module_path()? };
    for class in CLASSES {
        let clsid = clsid_key(&class.clsid);

        let key = unsafe { create_key(root, &format!("{base}\\CLSID\\{clsid}"))? };
        let r = (|| unsafe {
            set_default(key, class.description)?;
            // The whole point of the exercise: the path COM loads.
            let server = create_key(key, "InprocServer32")?;
            let r = (|| {
                set_default(server, &module.to_string())?;
                set_value(server, "ThreadingModel", "Both")
            })();
            close(server);
            r?;
            // Lets a caller say "Rustwinui3loop.NamedPipeConnector" instead of
            // carrying the GUID; `CLSIDFromProgID` reads this pair.
            let progid = create_key(key, "ProgID")?;
            let r = set_default(progid, class.progid);
            close(progid);
            r
        })();
        close(key);
        r?;

        let named = unsafe { create_key(root, &format!("{base}\\{}", class.progid))? };
        let r = (|| unsafe {
            set_default(named, class.description)?;
            let back = create_key(named, "CLSID")?;
            let r = set_default(back, &clsid);
            close(back);
            r
        })();
        close(named);
        r?;
    }

    Ok(())
}

/// The body of [`DllUnregisterServer`].
///
/// Deletes every class, and only then reports the first failure: a partial
/// registration should still be cleaned up as far as it can be, rather than
/// stopping at the first class that was already gone.
///
/// # Safety
///
/// `root` must be a live registry key the caller may write to.
pub(crate) unsafe fn unregister_under(root: HKEY, base: &str) -> Result<()> {
    let mut first_error = None;
    for class in CLASSES {
        for path in [
            format!("{base}\\CLSID\\{}", clsid_key(&class.clsid)),
            format!("{base}\\{}", class.progid),
        ] {
            let status = unsafe { RegDeleteTreeW(root, &HSTRING::from(&path)) };
            // A key that is not there is the state we wanted.
            if status.is_err()
                && status != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
                && first_error.is_none()
            {
                first_error = Some(HRESULT::from_win32(status.0));
            }
        }
    }
    match first_error {
        None => Ok(()),
        Some(e) => Err(windows_core::Error::from_hresult(e)),
    }
}

/// `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` — the braced form the registry
/// uses and `CLSIDFromString` parses.
pub(crate) fn clsid_key(g: &GUID) -> String {
    let d = g.data4;
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    )
}

/// The full path of whichever module contains this code.
///
/// Found from the address of a function here rather than from
/// `current_exe()` or a `DllMain`-captured handle, so it is right in both
/// shapes this crate builds: the DLL when COM loaded it, the executable when
/// the library was linked in statically (which is what the tests see).
///
/// # Safety
///
/// Reads a module handle without taking a reference on it, which is sound
/// only because the module in question is the one running this code.
unsafe fn module_path() -> Result<HSTRING> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_path as *const () as *const u16),
            &mut module,
        )?;
    }
    // Long paths are not the usual case but are entirely legal, and a
    // truncated server path is a module COM would fail to load with a
    // confusing error. Grow until it fits.
    let mut buf = vec![0u16; 260];
    loop {
        let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
        if n == 0 {
            return Err(windows_core::Error::from_thread());
        }
        if n < buf.len() {
            return Ok(HSTRING::from_wide(&buf[..n]));
        }
        if buf.len() >= 32768 {
            return Err(windows_core::Error::from_hresult(E_FAIL));
        }
        buf.resize(buf.len() * 2, 0);
    }
}

/// # Safety
/// `root` must be a live registry key the caller may write to.
unsafe fn create_key(root: HKEY, path: &str) -> Result<HKEY> {
    let mut key = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            &HSTRING::from(path),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if status.is_err() {
        return Err(windows_core::Error::from_hresult(HRESULT::from_win32(
            status.0,
        )));
    }
    Ok(key)
}

/// # Safety
/// `key` must be a live registry key opened for writing.
unsafe fn set_default(key: HKEY, value: &str) -> Result<()> {
    unsafe { set_raw(key, PCWSTR::null(), value) }
}

/// # Safety
/// `key` must be a live registry key opened for writing.
unsafe fn set_value(key: HKEY, name: &str, value: &str) -> Result<()> {
    let name = HSTRING::from(name);
    unsafe { set_raw(key, PCWSTR(name.as_ptr()), value) }
}

/// # Safety
/// `key` must be live and `name` must outlive the call.
unsafe fn set_raw(key: HKEY, name: PCWSTR, value: &str) -> Result<()> {
    // `REG_SZ` is counted in bytes and the count must include the
    // terminator, which `encode_utf16` does not emit.
    let mut bytes = Vec::with_capacity((value.len() + 1) * 2);
    for u in value.encode_utf16().chain(core::iter::once(0u16)) {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let status = unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(&bytes)) };
    if status.is_err() {
        return Err(windows_core::Error::from_hresult(HRESULT::from_win32(
            status.0,
        )));
    }
    Ok(())
}

/// Closes a key, discarding the status: there is nothing a caller could do
/// about a failed close, and swallowing it keeps the error that actually
/// matters — the one from the write — on the path out.
fn close(key: HKEY) {
    let _ = unsafe { RegCloseKey(key) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::{KEY_READ, RegOpenKeyExW, RegQueryValueExW};

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
                u.cast::<super::super::INamedPipeListenerBind>().is_ok()
            }),
            (CLSID_UNIX_CONNECTION_LISTENER, |u| {
                u.cast::<super::super::IUnixListenerBind>().is_ok()
            }),
            (CLSID_HYPER_V_CONNECTION_LISTENER, |u| {
                u.cast::<super::super::IHyperVListenerBind>().is_ok()
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
                DllGetClassObject(clsid, &IClassFactory::IID, &mut raw)
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
            unsafe { DllGetClassObject(&stranger, &IClassFactory::IID, &mut raw) },
            CLASS_E_CLASSNOTAVAILABLE
        );
        assert!(raw.is_null());

        // An interface the class object does not have.
        assert!(
            unsafe {
                DllGetClassObject(
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
            DllGetClassObject(&CLSID_UNIX_CONNECTOR, &IClassFactory::IID, &mut f)
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

    /// `DllRegisterServer`'s body against a scratch key, so the test says
    /// what the real call writes without touching the live class
    /// registrations — and leaves nothing behind either way.
    #[test]
    fn registration_writes_then_removes_every_key() {
        let base = format!("Software\\rustwinui3loop_test\\{}", std::process::id());
        // Whatever happens below, the scratch tree goes.
        struct Scrub(String);
        impl Drop for Scrub {
            fn drop(&mut self) {
                let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(&self.0)) };
            }
        }
        let _scrub = Scrub("Software\\rustwinui3loop_test".to_string());

        unsafe { register_under(HKEY_CURRENT_USER, &base) }.expect("register");

        let module = unsafe { module_path() }.expect("module_path").to_string();
        for class in CLASSES {
            let clsid = clsid_key(&class.clsid);
            // The value COM actually loads.
            assert_eq!(
                read_default(&format!("{base}\\CLSID\\{clsid}\\InprocServer32")).as_deref(),
                Some(module.as_str()),
                "{clsid} server path"
            );
            assert_eq!(
                read_value(
                    &format!("{base}\\CLSID\\{clsid}\\InprocServer32"),
                    "ThreadingModel"
                )
                .as_deref(),
                Some("Both")
            );
            // ProgID resolves to the CLSID and back.
            assert_eq!(
                read_default(&format!("{base}\\CLSID\\{clsid}\\ProgID")).as_deref(),
                Some(class.progid)
            );
            assert_eq!(
                read_default(&format!("{base}\\{}\\CLSID", class.progid)).as_deref(),
                Some(clsid.as_str())
            );
        }

        unsafe { unregister_under(HKEY_CURRENT_USER, &base) }.expect("unregister");
        for class in CLASSES {
            let clsid = clsid_key(&class.clsid);
            assert_eq!(read_default(&format!("{base}\\CLSID\\{clsid}")), None);
            assert_eq!(read_default(&format!("{base}\\{}", class.progid)), None);
        }

        // Removing what is already gone is success, not an error: `regsvr32
        // /u` on an unregistered module should not report a failure.
        unsafe { unregister_under(HKEY_CURRENT_USER, &base) }.expect("second unregister");
    }

    /// The shipped entry points must agree with what the docs promise, since
    /// the round-trip test above deliberately runs somewhere else.
    #[test]
    fn the_shipped_registration_is_per_user() {
        assert_eq!(CLASSES_ROOT, "Software\\Classes");
        assert_eq!(DllCanUnloadNow(), S_FALSE);
    }

    fn read_default(path: &str) -> Option<String> {
        read_raw(path, PCWSTR::null())
    }

    fn read_value(path: &str, name: &str) -> Option<String> {
        let name = HSTRING::from(name);
        read_raw(path, PCWSTR(name.as_ptr()))
    }

    fn read_raw(path: &str, name: PCWSTR) -> Option<String> {
        unsafe {
            let mut key = HKEY::default();
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(path),
                None,
                KEY_READ,
                &mut key,
            )
            .ok()
            .ok()?;
            let mut bytes = 0u32;
            let status = RegQueryValueExW(key, name, None, None, None, Some(&mut bytes));
            if status.is_err() {
                close(key);
                return None;
            }
            let mut buf = vec![0u8; bytes as usize];
            let status = RegQueryValueExW(
                key,
                name,
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut bytes),
            );
            close(key);
            status.ok().ok()?;
            let (pairs, _) = buf.as_chunks::<2>();
            let units: Vec<u16> = pairs
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .take_while(|&u| u != 0)
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
    }
}
