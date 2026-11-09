//! The activation path with the COM loader actually in it.
//!
//! `comio::activation`'s own unit tests call `DllGetClassObject` directly,
//! which covers the class factories but skips the half that only exists
//! outside the process: the registry lookup, `LoadLibrary` on the path it
//! found, and `GetProcAddress` for the entry point. Those need a real DLL,
//! so they live here, where the crate has been built as a `cdylib` as well
//! as an rlib.
//!
//! What this proves that nothing else can: a caller holding only a CLSID and
//! `CoCreateInstance` — which is every COM-capable language — gets working
//! objects. The objects come from the DLL's copy of the code while this test
//! runs the rlib's copy, so the two halves meet only across the COM ABI,
//! exactly as a C++ or C# caller would meet them.
#![cfg(feature = "comio")]

use rustwinui3loop::comio::activation::{
    CLSID_NAMED_PIPE_CONNECTOR, CLSID_NAMED_PIPE_CONNECTION_LISTENER,
};
use rustwinui3loop::comio::bindings::Rustwinui3loop::{
    ConnectionReceivedArgs, IConnectionListener, INamedPipeConnector, INamedPipeBind,
    ITransport, TransportStream,
};
use std::sync::mpsc;
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{E_INVALIDARG, FreeLibrary, HMODULE, S_FALSE, S_OK};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::System::ApplicationInstallationAndServicing::{
    ACTCTXW, ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx,
};
use windows::Win32::System::WinRT::{
    IActivationFactory, RO_INIT_MULTITHREADED, RoActivateInstance, RoInitialize,
};
use windows_core::PCWSTR;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CLSIDFromProgID, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{HRESULT, HSTRING, IUnknown, Interface, PCSTR};

type DllEntry = unsafe extern "system" fn() -> HRESULT;

/// Where the cdylib lands.
///
/// `current_exe()` is `target/<profile>/deps/activation-<hash>.exe`, and the
/// DLL sits one level up. Deriving it from the running binary rather than
/// from `CARGO_MANIFEST_DIR` keeps this right under `--release` and under a
/// redirected `CARGO_TARGET_DIR`.
fn profile_dir() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent()
        .and_then(|deps| deps.parent())
        .expect("target dir")
        .to_path_buf()
}

/// Builds the cdylib, because `cargo test` does not.
///
/// Nothing in the dependency graph links the DLL — the test harness uses the
/// rlib — so cargo has no reason to produce it, and a plain `cargo test` on
/// a clean tree would otherwise fail here for a reason that has nothing to
/// do with the code under test. One nested `cargo build --lib` fixes that;
/// cargo releases its lock on the target directory before running test
/// binaries, so this does not deadlock, and on an already-built tree it is
/// a no-op costing a fraction of a second.
///
/// The feature list is reconstructed from this crate's own `cfg`s rather
/// than hardcoded: features are package-wide, so building the library with a
/// different set than the test was compiled against would make cargo rebuild
/// it on every alternating run.
fn ensure_dll() -> std::path::PathBuf {
    let dir = profile_dir();
    let mut features = vec!["comio"];
    for (on, name) in [
        (cfg!(feature = "tp"), "tp"),
        (cfg!(feature = "dq"), "dq"),
        (cfg!(feature = "rio"), "rio"),
        (cfg!(feature = "futures-io"), "futures-io"),
    ] {
        if on {
            features.push(name);
        }
    }

    let mut cmd = std::process::Command::new(env!("CARGO"));
    cmd.args([
        "build",
        "--lib",
        "--no-default-features",
        "--features",
        &features.join(","),
    ])
    .current_dir(env!("CARGO_MANIFEST_DIR"));
    if dir.file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    let out = cmd.output().expect("spawn cargo build");
    assert!(
        out.status.success(),
        "building the cdylib failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let path = dir.join("rustwinui3loop.dll");
    assert!(
        path.exists(),
        "{} missing after a successful build — is `crate-type` still [\"lib\", \"cdylib\"]?",
        path.display()
    );
    path
}

/// Loads the DLL and calls one of its exported `Dll*` entry points.
///
/// Going through `GetProcAddress` rather than calling the statically linked
/// copy is the whole point: the registration has to be performed *by the
/// module being registered*, because `DllRegisterServer` writes the path of
/// whichever module it runs in. Called from the rlib's copy it would
/// register this test executable, which COM cannot load.
unsafe fn call_entry(module: HMODULE, name: &[u8]) -> HRESULT {
    let addr = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }
        .unwrap_or_else(|| panic!("{} not exported", String::from_utf8_lossy(name)));
    let f: DllEntry = unsafe { core::mem::transmute(addr) };
    unsafe { f() }
}

/// Unregisters on the way out, however the test ends.
///
/// This is the one test in the tree that writes to the live registry
/// (`HKEY_CURRENT_USER\Software\Classes`, under this crate's own CLSIDs —
/// no elevation, nothing machine-wide). A panic mid-test still runs this,
/// so a failed run does not leave class registrations pointing at a
/// `target/debug` path that may later be deleted.
struct Registered(HMODULE);

impl Drop for Registered {
    fn drop(&mut self) {
        let hr = unsafe { call_entry(self.0, b"DllUnregisterServer\0") };
        if hr.is_err() {
            // Cannot panic here — this may already be unwinding — but a
            // silent failure would leave class registrations behind that
            // point at a build directory, so at least say so.
            eprintln!("DllUnregisterServer failed: {hr:?}; HKCU class keys may remain");
        }
    }
}

/// One test rather than several: registration is process-wide and
/// `cargo test` runs integration tests in parallel threads, so two tests
/// each registering and unregistering the same CLSIDs would race — one
/// tearing down the keys the other is mid-activation on.
#[test]
fn com_activation_round_trip_through_the_registered_dll() {
    let path = ensure_dll();

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .expect("CoInitializeEx");

    let module = unsafe { LoadLibraryW(&HSTRING::from(path.as_os_str())) }.expect("LoadLibraryW");
    // `S_OK` means both halves registered; `S_FALSE` means the machine-wide
    // WinRT half was denied because this run is not elevated, leaving only
    // the per-user COM half in place. Both are successes, and which one came
    // back has to agree with what is actually in the registry — so a silent
    // half-registration cannot pass.
    let registered = unsafe { call_entry(module, b"DllRegisterServer\0") };
    assert!(
        registered == S_OK || registered == S_FALSE,
        "DllRegisterServer: {registered:?}"
    );
    let _registered = Registered(module);
    assert_eq!(
        winrt_key_present("Rustwinui3loop.NamedPipeConnector"),
        registered == S_OK,
        "DllRegisterServer said {registered:?} but the registry disagrees"
    );

    // --- The listener, created the way any COM caller would ---------------
    // The failure message carries the registration state, because
    // `REGDB_E_CLASSNOTREG` here means the keys this test just wrote are not
    // the keys COM read — which hive the DLL registered into is the first
    // thing worth knowing, and an elevated `regsvr32` run beforehand writes
    // into a different one.
    let listener: IUnknown =
        unsafe { CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTION_LISTENER, None, CLSCTX_INPROC_SERVER) }
            .unwrap_or_else(|e| {
                panic!(
                    "CoCreateInstance(listener): {e:?}
                       DllRegisterServer returned {registered:?}
                       HKCU CLSID key present: {}
                       HKLM ActivatableClassId key present: {}",
                    com_key_present(&CLSID_NAMED_PIPE_CONNECTION_LISTENER),
                    winrt_key_present("Rustwinui3loop.NamedPipeConnector"),
                )
            });

    let name = format!(r"\\.\pipe\comio_activation_{}", std::process::id());
    // The block below used to be raw-pointer out-params, `ManuallyDrop` and
    // hand-checked `HRESULT`s. The generated WinRT bindings carry the result
    // in the return value and the failure in an `Err`, so what is left is the
    // test rather than the marshalling.
    // The raw pointer rather than the object: a WinRT interface wrapper is
    // not `Send`, and this crosses from the thread-pool callback thread to
    // the test's. Ownership transfers with it.
    let (accepted_tx, accepted_rx) = mpsc::channel::<usize>();
    let handler = TypedEventHandler::<IConnectionListener, ConnectionReceivedArgs>::new(
        move |_sender, args| {
            let stream = args.as_ref().unwrap().Stream()?;
            let _ = accepted_tx.send(stream.into_raw() as usize);
            Ok(())
        },
    );

    let bind: INamedPipeBind = listener.cast().expect("QI INamedPipeBind");
    bind.BindAsync(&HSTRING::from(&name)).expect("BindAsync");

    let events: IConnectionListener = listener.cast().expect("QI IConnectionListener");
    events.ConnectionReceived(&handler).expect("ConnectionReceived");

    // --- No ProgID is registered any more ---------------------------------
    // It used to be, and `CLSIDFromProgID` resolving it was a second,
    // independent read of what registration wrote. The key is gone on
    // purpose: a ProgID is for script hosts reaching the class through
    // `IDispatch`, which these objects no longer implement, so resolving one
    // would hand a script an object it could create and never call. Asserting
    // the *absence* keeps that deliberate rather than accidental.
    assert_eq!(
        unsafe { CLSIDFromProgID(&HSTRING::from("Rustwinui3loop.NamedPipeConnector")) }
            .map(|_| ())
            .map_err(|e| e.code()),
        Err(CO_E_CLASSSTRING),
        "a ProgID key came back; registration is advertising IDispatch again"
    );

    // --- The client, by CLSID ---------------------------------------------
    let client: IUnknown =
        unsafe { CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTOR, None, CLSCTX_INPROC_SERVER) }
            .expect("CoCreateInstance(client)");

    // The shared interface answers for a COM-activated object too.
    let connector: ITransport = client.cast().expect("QI ITransport");
    assert_eq!(
        connector.Transport().expect("Transport").to_string(),
        "namedpipe"
    );

    let connect: INamedPipeConnector = client.cast().expect("QI INamedPipeConnector");
    let connected = connect
        .ConnectAsync(&HSTRING::from(&name))
        .expect("ConnectAsync")
        .join()
        .expect("connect result");

    // --- Both ends are real streams ---------------------------------------
    let server_raw = accepted_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server =
        unsafe { TransportStream::from_raw(server_raw as *mut core::ffi::c_void) };
    assert!(
        server
            .cast::<windows::Storage::Streams::IInputStream>()
            .is_ok()
    );
    assert!(
        connected
            .cast::<windows::Storage::Streams::IOutputStream>()
            .is_ok()
    );

    let closable: windows::Foundation::IClosable = events.cast().unwrap();
    closable.Close().unwrap();

    // --- And the WinRT half of activation --------------------------------
    winrt_activation_factory(module);
    ro_activate_instance_reg_free(&path);

    // The module stays loaded on purpose: `DllCanUnloadNow` returns S_FALSE,
    // and objects created above are still alive in this scope. Dropping the
    // reference `LoadLibraryW` took is all this undoes.
    drop(_registered);
    unsafe { FreeLibrary(module) }.expect("FreeLibrary");
}

/// The `DllGetActivationFactory` export, exercised exactly as the Windows
/// Runtime would call it.
///
/// Deliberately *not* going through `RoActivateInstance`: that would need the
/// `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId` registration,
/// which needs elevation and is not written yet. Calling the export directly
/// tests everything on this side of that registry read — the name lookup, the
/// factory, `ActivateInstance`, and the identity of what comes back — and
/// leaves only the registry entry itself unproven.
///
/// Each class is asked for by the name the metadata gives it, and the object
/// that comes back is asked what it is. The two have to agree: the lookup
/// table and `GetRuntimeClassName` both read the same constant, so a class
/// renamed in only one place fails here.
fn winrt_activation_factory(module: HMODULE) {
    type GetActivationFactory = unsafe extern "system" fn(
        core::mem::ManuallyDrop<HSTRING>,
        *mut *mut core::ffi::c_void,
    ) -> HRESULT;

    let addr = unsafe { GetProcAddress(module, PCSTR(c"DllGetActivationFactory".as_ptr().cast())) }
        .expect("DllGetActivationFactory not exported");
    let get_factory: GetActivationFactory = unsafe { core::mem::transmute(addr) };

    let factory_for = |name: &str| -> Result<IActivationFactory, HRESULT> {
        let mut raw = core::ptr::null_mut();
        let hr = unsafe {
            get_factory(
                core::mem::ManuallyDrop::new(HSTRING::from(name)),
                &mut raw,
            )
        };
        if hr.is_ok() {
            Ok(unsafe { IActivationFactory::from_raw(raw) })
        } else {
            assert!(raw.is_null(), "{name}: failed but still wrote a pointer");
            Err(hr)
        }
    };

    for name in [
        "Rustwinui3loop.NamedPipeConnectionListener",
        "Rustwinui3loop.UnixConnectionListener",
        "Rustwinui3loop.HyperVConnectionListener",
        "Rustwinui3loop.NamedPipeConnector",
        "Rustwinui3loop.UnixConnector",
        "Rustwinui3loop.HyperVConnector",
    ] {
        let factory = factory_for(name).unwrap_or_else(|hr| panic!("{name}: {hr:?}"));
        let instance = unsafe { factory.ActivateInstance() }
            .unwrap_or_else(|e| panic!("{name}: ActivateInstance: {e:?}"));
        assert_eq!(
            instance
                .GetRuntimeClassName()
                .unwrap_or_else(|e| panic!("{name}: GetRuntimeClassName: {e:?}"))
                .to_string(),
            name,
            "activated object does not answer to the name it was asked for"
        );
    }

    // Classes that exist in the metadata but are not `#[Activatable]`, so the
    // factory must refuse them rather than quietly make one.
    for name in [
        "Rustwinui3loop.TransportStream",
        "Rustwinui3loop.ConnectionReceivedArgs",
        "Rustwinui3loop.NoSuchClass",
        "Windows.Storage.Streams.Buffer",
    ] {
        assert_eq!(
            factory_for(name).err(),
            Some(REGDB_E_CLASSNOTREG),
            "{name} should not be activatable here"
        );
    }

    // The documented answer for a null argument, both ways round.
    let mut raw = core::ptr::null_mut();
    assert_eq!(
        unsafe {
            get_factory(
                core::mem::ManuallyDrop::new(HSTRING::new()),
                &mut raw,
            )
        },
        E_INVALIDARG
    );
    assert_eq!(
        unsafe {
            get_factory(
                core::mem::ManuallyDrop::new(HSTRING::from("Rustwinui3loop.NamedPipeConnector")),
                core::ptr::null_mut(),
            )
        },
        E_INVALIDARG
    );
}

/// The last step of the WinRT path: `RoActivateInstance` by name, with the
/// runtime finding the module itself.
///
/// Reg-free rather than through the registry, because the machine-wide route
/// is closed to us: `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`
/// is owned by `NT SERVICE\TrustedInstaller` and Administrators have read
/// access only, so even an elevated `DllRegisterServer` is denied. A manifest
/// loaded into an activation context needs no elevation at all, and is what
/// a consumer that cannot edit its own host's manifest — `python.exe`, say —
/// would use.
///
/// The control matters here: the same two calls are made *before* the
/// activation context is active and must fail, so a pass cannot be an
/// accident of something else having registered these classes.
fn ro_activate_instance_reg_free(dll: &Path) {
    const NAMES: [&str; 2] = [
        "Rustwinui3loop.NamedPipeConnector",
        "Rustwinui3loop.NamedPipeConnectionListener",
    ];

    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.expect("RoInitialize");

    // Control: without the manifest there is nothing to find.
    for name in NAMES {
        let hr = unsafe { RoActivateInstance(&HSTRING::from(name)) }
            .err()
            .map(|e| e.code());
        assert_eq!(
            hr,
            Some(REGDB_E_CLASSNOTREG),
            "{name} activated with no activation context; something else has it registered"
        );
    }

    // The manifest is generated from the same `CLASSES` table the activation
    // factory looks names up in, so the two cannot disagree.
    let dir = dll.parent().expect("dll directory");
    let manifest_path = dir.join("rustwinui3loop.manifest");
    std::fs::write(&manifest_path, rustwinui3loop::comio::activation::manifest())
        .expect("write manifest");

    let source: Vec<u16> = manifest_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ctx = ACTCTXW {
        cbSize: core::mem::size_of::<ACTCTXW>() as u32,
        lpSource: PCWSTR(source.as_ptr()),
        ..Default::default()
    };
    let handle = unsafe { CreateActCtxW(&ctx) }.expect("CreateActCtxW");
    let mut cookie = 0usize;
    unsafe { ActivateActCtx(Some(handle), &mut cookie) }.expect("ActivateActCtx");

    for name in NAMES {
        let instance = unsafe { RoActivateInstance(&HSTRING::from(name)) }
            .unwrap_or_else(|e| panic!("RoActivateInstance({name}): {e:?}"));
        assert_eq!(
            instance
                .GetRuntimeClassName()
                .expect("GetRuntimeClassName")
                .to_string(),
            name,
            "activated object does not answer to the name it was asked for"
        );
    }

    // And it is the real object, not merely something named right.
    let connector: ITransport =
        unsafe { RoActivateInstance(&HSTRING::from(NAMES[0])) }
            .expect("RoActivateInstance")
            .cast()
            .expect("QI ITransport");
    assert_eq!(
        connector.Transport().expect("Transport").to_string(),
        "namedpipe"
    );

    unsafe { DeactivateActCtx(Default::default(), cookie) }.expect("DeactivateActCtx");
    unsafe { ReleaseActCtx(handle) };
}

const WINRT_ROOT: &str = "SOFTWARE\\Microsoft\\WindowsRuntime\\ActivatableClassId";

/// Whether the per-user COM registration for `clsid` exists — the key
/// `CoCreateInstance` would have read.
fn com_key_present(clsid: &windows_core::GUID) -> bool {
    let d = clsid.data4;
    let path = format!(
        "Software\\Classes\\CLSID\\{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}\\InprocServer32",
        clsid.data1, clsid.data2, clsid.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
    );
    unsafe {
        let mut key = HKEY::default();
        let opened =
            RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut key)
                .is_ok();
        if opened {
            let _ = RegCloseKey(key);
        }
        opened
    }
}

/// Whether the machine registration for `name` exists, read directly rather
/// than inferred from an elevation check.
fn winrt_key_present(name: &str) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let path = HSTRING::from(format!("{WINRT_ROOT}\\{name}"));
        let opened = RegOpenKeyExW(HKEY_LOCAL_MACHINE, &path, None, KEY_READ, &mut key).is_ok();
        if opened {
            let _ = RegCloseKey(key);
        }
        opened
    }
}

/// REGDB_E_CLASSNOTREG, as `DllGetActivationFactory` documents it.
const REGDB_E_CLASSNOTREG: HRESULT = HRESULT(0x80040154u32 as i32);

/// CO_E_CLASSSTRING — what `CLSIDFromProgID` answers for a name that resolves
/// to nothing. Not `REGDB_E_CLASSNOTREG`: that one means "the CLSID is known
/// but unregistered", and here the lookup fails a step earlier, at the name.
const CO_E_CLASSSTRING: HRESULT = HRESULT(0x800401F3u32 as i32);
