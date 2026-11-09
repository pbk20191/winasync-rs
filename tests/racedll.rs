//! A host that unloads the server without asking.
//!
//! `DllCanUnloadNow` is advisory. A host may `FreeLibrary` the DLL while an
//! operation is still in flight, and the completion callback -- a function
//! pointer into the module -- then has nowhere safe to go. This test does
//! exactly that and requires the process to survive.
//!
//! # What `RaceDll` does, measured
//!
//! `comio::pool::callback_env` sets `RaceDll` (what
//! `SetThreadpoolCallbackLibrary` sets) on every environment the server uses.
//! The obvious reading -- the pool keeps the DLL loaded while I/O is
//! outstanding -- is **wrong**, and this test was first written to that
//! reading and failed. Run against the same scenario:
//!
//! | | after `FreeLibrary` | when the read later completes |
//! | --- | --- | --- |
//! | without `RaceDll` | unmapped | `STATUS_ACCESS_VIOLATION` |
//! | with `RaceDll` | unmapped | survives |
//!
//! So the module is unmapped either way. What `RaceDll` buys is what its name
//! says: the pool settles the race between a callback and the unload, and
//! does not call into a module that is gone. The operation block is leaked
//! instead, which is the right trade in a host that has already broken the
//! protocol.
//!
//! # Release builds only
//!
//! In a debug build the server's `DllMain` treats this exact host behaviour
//! as a bug and aborts on detach, naming what was still in flight -- which is
//! how the measurement above was found. That is the other half of the design:
//! loud in development, survivable in release. This test is about the second
//! half, so it runs under `cargo test --release`.
#![cfg(all(feature = "comio", not(debug_assertions)))]

use std::time::Duration;
use winasync::comio::bindings::Pbk20191::WinAsync::Comio::{INamedPipeConnector, ITransportStream};
use windows::Storage::Streams::{Buffer, IInputStream, InputStreamOptions};
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HMODULE, S_FALSE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE};
use windows::Win32::System::WinRT::IActivationFactory;
use windows_core::{HRESULT, HSTRING, IInspectable, Interface, PCSTR, w};

#[test]
fn a_late_completion_after_a_forced_unload_does_not_call_into_freed_code() {
    let path = build_dll();
    assert!(
        unsafe { GetModuleHandleW(w!("winasync.dll")) }.is_err(),
        "precondition: the server DLL is not already loaded"
    );
    let module = unsafe { LoadLibraryW(&HSTRING::from(path.as_os_str())) }.expect("LoadLibraryW");

    // The other end belongs to the test, so it stays open -- and the read
    // stays pending -- until the test decides otherwise.
    let pipe = format!(r"\\.\pipe\comio_racedll_{}", std::process::id());
    let server = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(&pipe),
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!server.is_invalid(), "CreateNamedPipeW failed");

    // Everything below comes out of the DLL, by name, the way the Windows
    // Runtime would ask for it -- not from this test's statically linked copy.
    {
        let connector: INamedPipeConnector = unsafe { activate(module, "Pbk20191.WinAsync.Comio.NamedPipeConnector") }
            .cast()
            .expect("QI INamedPipeConnector");
        let stream: ITransportStream = connector
            .ConnectAsync(&HSTRING::from(&pipe))
            .expect("ConnectAsync")
            .join()
            .expect("connect");
        let buffer = Buffer::Create(16).expect("Buffer::Create");
        let _read = stream
            .cast::<IInputStream>()
            .expect("QI IInputStream")
            .ReadAsync(&buffer, 16, InputStreamOptions::Partial)
            .expect("ReadAsync");
        // Every object is released here; the read is still pending, because
        // nothing has been written.
    }

    assert_eq!(
        unsafe { can_unload_now(module) },
        S_FALSE,
        "the module offered to unload with a read in flight"
    );

    // The host ignores that answer and unloads anyway. The module is unmapped
    // here, `RaceDll` or not -- that is not what it protects against.
    unsafe { FreeLibrary(module) }.expect("FreeLibrary");

    // Now the read completes, and the pool has a callback for a module that
    // no longer exists. Without `RaceDll` this is an access violation and the
    // test binary dies; with it, the pool declines to call. Reaching the end
    // of this function is the assertion.
    unsafe { CloseHandle(server) }.expect("CloseHandle");
    std::thread::sleep(Duration::from_millis(500));
}

/// Activates a runtime class through the DLL's own `DllGetActivationFactory`.
///
/// # Safety
///
/// `module` must be the loaded server DLL.
unsafe fn activate(module: HMODULE, class: &str) -> IInspectable {
    type GetActivationFactory =
        unsafe extern "system" fn(core::mem::ManuallyDrop<HSTRING>, *mut *mut core::ffi::c_void) -> HRESULT;
    let addr = unsafe { GetProcAddress(module, PCSTR(c"DllGetActivationFactory".as_ptr().cast())) }
        .expect("DllGetActivationFactory not exported");
    let get: GetActivationFactory = unsafe { core::mem::transmute(addr) };
    let name = HSTRING::from(class);
    let mut raw = core::ptr::null_mut();
    unsafe { get(core::mem::ManuallyDrop::new(name.clone()), &mut raw) }
        .ok()
        .expect("DllGetActivationFactory");
    let factory = unsafe { IActivationFactory::from_raw(raw) };
    unsafe { factory.ActivateInstance() }.expect("ActivateInstance")
}

/// # Safety
///
/// `module` must be the loaded server DLL.
unsafe fn can_unload_now(module: HMODULE) -> HRESULT {
    let addr = unsafe { GetProcAddress(module, PCSTR(c"DllCanUnloadNow".as_ptr().cast())) }
        .expect("DllCanUnloadNow not exported");
    let f: unsafe extern "system" fn() -> HRESULT = unsafe { core::mem::transmute(addr) };
    unsafe { f() }
}

/// Builds the server DLL; see `tests/activation.rs`'s `ensure_dll` for why a
/// nested build is needed and safe.
fn build_dll() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target dir")
        .to_path_buf();
    let mut cmd = std::process::Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "winasync-dll"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if dir.file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    let out = cmd.output().expect("spawn cargo build");
    assert!(
        out.status.success(),
        "building the server DLL failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir.join("winasync.dll")
}
