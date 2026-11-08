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
    CLSID_NAMED_PIPE_CLIENT, CLSID_NAMED_PIPE_CONNECTION_LISTENER,
};
use rustwinui3loop::comio::{
    IConnectionListener, IConnectionReceivedArgs, INamedPipeConnector, INamedPipeListenerBind,
    IStreamConnector,
};
use std::sync::mpsc;
use windows::Foundation::TypedEventHandler;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CLSIDFromProgID, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{HRESULT, HSTRING, IInspectable, IUnknown, Interface, PCSTR};
use windows_future::IAsyncOperation;

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
    unsafe { call_entry(module, b"DllRegisterServer\0") }
        .ok()
        .expect("DllRegisterServer");
    let _registered = Registered(module);

    // --- The listener, created the way any COM caller would ---------------
    let listener: IUnknown =
        unsafe { CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTION_LISTENER, None, CLSCTX_INPROC_SERVER) }
            .expect("CoCreateInstance(listener)");

    let name = format!(r"\\.\pipe\comio_activation_{}", std::process::id());
    let (accepted_tx, accepted_rx) = mpsc::channel::<usize>();
    let handler = TypedEventHandler::<IInspectable, IInspectable>::new(move |_s, args| {
        let args: IConnectionReceivedArgs = args.as_ref().unwrap().cast().unwrap();
        let raw = unsafe {
            let mut raw = core::ptr::null_mut();
            args.Stream(&mut raw).ok()?;
            raw as usize
        };
        let _ = accepted_tx.send(raw);
        Ok(())
    });

    let bind: INamedPipeListenerBind = listener.cast().expect("QI INamedPipeListenerBind");
    unsafe {
        let mut action = core::ptr::null_mut();
        bind.BindAsync(core::mem::ManuallyDrop::new(HSTRING::from(&name)), &mut action)
            .ok()
            .expect("BindAsync");
        drop(windows_future::IAsyncAction::from_raw(action));
    }

    let events: IConnectionListener = listener.cast().expect("QI IConnectionListener");
    unsafe {
        let mut token = 0i64;
        events
            .ConnectionReceived(handler.as_raw(), &mut token)
            .ok()
            .expect("ConnectionReceived");
    }

    // --- The client, reached by ProgID rather than by CLSID ---------------
    // `CLSIDFromProgID` is a second, independent read of what registration
    // wrote: it resolves only if the ProgID key and its CLSID subkey are
    // both right.
    let from_progid = unsafe {
        CLSIDFromProgID(&HSTRING::from("Rustwinui3loop.NamedPipeClient"))
    }
    .expect("CLSIDFromProgID");
    assert_eq!(from_progid, CLSID_NAMED_PIPE_CLIENT);

    let client: IUnknown = unsafe { CoCreateInstance(&from_progid, None, CLSCTX_INPROC_SERVER) }
        .expect("CoCreateInstance(client)");

    // The shared interface answers for a COM-activated object too.
    let connector: IStreamConnector = client.cast().expect("QI IStreamConnector");
    let transport = unsafe {
        let mut s = core::mem::ManuallyDrop::new(HSTRING::new());
        connector.Transport(&mut s).ok().expect("Transport");
        core::mem::ManuallyDrop::into_inner(s).to_string()
    };
    assert_eq!(transport, "namedpipe");

    let connect: INamedPipeConnector = client.cast().expect("QI INamedPipeConnector");
    let connected: IInspectable = unsafe {
        let mut op = core::ptr::null_mut();
        connect
            .ConnectAsync(core::mem::ManuallyDrop::new(HSTRING::from(&name)), &mut op)
            .ok()
            .expect("ConnectAsync");
        IAsyncOperation::<IInspectable>::from_raw(op)
    }
    .join()
    .expect("connect result");

    // --- Both ends are real streams ---------------------------------------
    let server_raw = accepted_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("no ConnectionReceived");
    let server = unsafe { IInspectable::from_raw(server_raw as *mut core::ffi::c_void) };
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

    // The module stays loaded on purpose: `DllCanUnloadNow` returns S_FALSE,
    // and objects created above are still alive in this scope. Dropping the
    // reference `LoadLibraryW` took is all this undoes.
    drop(_registered);
    unsafe { FreeLibrary(module) }.expect("FreeLibrary");
}
