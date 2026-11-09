//! The out-of-process server, with a real process boundary in the way.
//!
//! `tests/activation.rs` proves the in-process server: the DLL is loaded into
//! this test and called directly. Here the objects live in
//! `winasync-server.exe`, which COM starts on demand, and every call -- the
//! event a listener raises back into this process included -- crosses the
//! boundary through the proxy/stub in `winasync_ps.dll`. A pass means the
//! surface is complete across processes, not merely that one method happens
//! to marshal.
//!
//! The test plays the installer, as `tests/activation.rs` does: it writes the
//! `LocalServer32`, `ProxyStubClsid32` and proxy `InprocServer32` keys under
//! `HKEY_CURRENT_USER\Software\Classes` from the data `comio::activation`
//! publishes, and removes them however the test ends. No elevation.
//!
//! One `#[test]`, because the registration and the server process are both
//! shared state; two tests in parallel would tear each other's keys down.
#![cfg(feature = "comio")]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use winasync::comio::activation::{
    CLASSES, CLSID_NAMED_PIPE_CONNECTION_LISTENER, CLSID_NAMED_PIPE_CONNECTOR, CLSID_PROXY_STUB,
    PROXIED_IIDS, clsid_key,
};
use winasync::comio::bindings::Pbk20191::WinAsync::Comio::{
    IConnectionListener, IConnectionReceivedArgs, INamedPipeBind, INamedPipeConnector, ITransport,
    ITransportStream,
};
use windows::Foundation::{IClosable, TypedEventHandler};
use windows_future::{AsyncStatus, IAsyncInfo};
use windows::Storage::Streams::{
    Buffer, IBuffer, IBuffer_Impl, IInputStream, IOutputStream, InputStreamOptions,
};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CloseHandle, E_INVALIDARG, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IClientSecurity,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::System::WinRT::{IBufferByteAccess, IBufferByteAccess_Impl};
use windows_core::{AgileReference, GUID, HRESULT, HSTRING, IInspectable, IUnknown, Interface, PCSTR, PCWSTR, implement};

const SERVER_EXE: &str = "winasync-server.exe";
const PROXY_DLL: &str = "winasync_ps.dll";

/// How long the server gets to exit after its last reference goes. Generous:
/// the release crosses the process, and the listener's pending accept has to
/// be cancelled and complete before the count can reach zero.
const EXIT_WITHIN: Duration = Duration::from_secs(15);

#[test]
fn every_interface_works_across_the_process_boundary() {
    let dir = build();
    let server = dir.join(SERVER_EXE);
    let proxy = dir.join(PROXY_DLL);

    proxy_dll_answers_to_its_clsid(&proxy);

    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .expect("CoInitializeEx");
    assert!(
        server_pids(&server).is_empty(),
        "a {SERVER_EXE} from this build is already running; a stale server would \
         answer this test's activations"
    );
    let _registered = Registered::install(&server, &proxy);

    let pid = round_trip(&server);
    wait_for_exit(pid, "after the last reference was released");

    server_death_is_an_error_not_a_crash(&server);
}

/// The proxy/stub DLL's identity, checked directly before anything relies on
/// it: `ps/build.rs` defines `PROXY_CLSID_IS` and `comio::activation`
/// defines `CLSID_PROXY_STUB`, and they must be the same GUID.
///
/// The RPC runtime's `NdrDllGetClassObject` answers to that CLSID *and* to
/// the IID of every interface it proxies, so only the first is evidence of
/// the define: `CLSID_PROXY_STUB` is not one of the IIDs, and without
/// `PROXY_CLSID_IS` nothing in the DLL would know it. The control is a GUID
/// that is neither.
fn proxy_dll_answers_to_its_clsid(proxy: &Path) {
    // IPSFactoryBuffer: what COM asks a proxy/stub DLL's class object for.
    const IID_IPSFACTORYBUFFER: GUID = GUID::from_u128(0xd5f569d0_593b_101a_b569_08002b2dbf7a);
    type GetClassObject =
        unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT;

    let module = unsafe { LoadLibraryW(&HSTRING::from(proxy.as_os_str())) }.expect("load proxy DLL");
    let addr = unsafe { GetProcAddress(module, PCSTR(c"DllGetClassObject".as_ptr().cast())) }
        .expect("DllGetClassObject not exported by the proxy DLL");
    let get: GetClassObject = unsafe { core::mem::transmute(addr) };

    let mut raw = core::ptr::null_mut();
    let hr = unsafe { get(&CLSID_PROXY_STUB, &IID_IPSFACTORYBUFFER, &mut raw) };
    assert!(hr.is_ok(), "the proxy DLL refused CLSID_PROXY_STUB: {hr:?}");
    drop(unsafe { IUnknown::from_raw(raw) });

    const UNRELATED: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    let mut raw = core::ptr::null_mut();
    let hr = unsafe { get(&UNRELATED, &IID_IPSFACTORYBUFFER, &mut raw) };
    assert_eq!(hr, CLASS_E_CLASSNOTAVAILABLE, "control: the proxy DLL answers to anything");
    // Left loaded: COM will load the same DLL for the proxies below anyway.
}

/// Everything a client does with the six classes, out of process. Returns the
/// server's PID so the caller can watch it exit.
fn round_trip(server: &Path) -> u32 {
    let name = format!(r"\\.\pipe\comio_oop_{}", std::process::id());

    let listener: IConnectionListener = unsafe {
        CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTION_LISTENER, None, CLSCTX_LOCAL_SERVER)
    }
    .expect("CoCreateInstance(CLSCTX_LOCAL_SERVER) for the listener");

    // --- 1. It is a proxy, and it is in another process ------------------
    // `IClientSecurity` is implemented by the proxy manager and by nothing
    // else, so it separates "an object from another process" from "an object
    // that was created here after all". The control is a local object.
    assert!(
        listener.cast::<IClientSecurity>().is_ok(),
        "the listener is not a proxy: CLSCTX_LOCAL_SERVER created it in-process"
    );
    assert!(
        winasync::comio::NamedPipeConnectionListener::new()
            .cast::<IClientSecurity>()
            .is_err(),
        "control: a local object must not answer IClientSecurity"
    );
    let pids = server_pids(server);
    assert_eq!(pids.len(), 1, "expected exactly one server process, found {pids:?}");
    let pid = pids[0];
    assert_ne!(pid, std::process::id());

    // --- 2. Identity crosses ----------------------------------------------
    assert_eq!(
        listener.cast::<IInspectable>().unwrap().GetRuntimeClassName().unwrap().to_string(),
        "Pbk20191.WinAsync.Comio.NamedPipeConnectionListener"
    );
    assert_eq!(
        listener.cast::<ITransport>().unwrap().Transport().unwrap().to_string(),
        "namedpipe"
    );

    // --- 3. An event raised in the server runs here -----------------------
    // Through an `AgileReference`: the projected interface is not `Send`,
    // and the handler runs on whatever thread RPC delivers the call on.
    let (accepted_tx, accepted_rx) = mpsc::channel::<AgileReference<ITransportStream>>();
    let accepted_tx = std::sync::Mutex::new(accepted_tx);
    let token = listener
        .ConnectionReceived(&TypedEventHandler::<IConnectionListener, IConnectionReceivedArgs>::new(
            move |_sender, args| {
                let stream = args.as_ref().expect("args").Stream()?;
                let _ = accepted_tx.lock().unwrap().send(AgileReference::new(&stream)?);
                Ok(())
            },
        ))
        .expect("add ConnectionReceived");
    listener
        .cast::<INamedPipeBind>()
        .unwrap()
        .BindAsync(&HSTRING::from(&name))
        .expect("BindAsync")
        .join()
        .expect("bind");

    // The connector is a second activation, served by the same process:
    // the class objects are registered `REGCLS_MULTIPLEUSE`.
    let connector: INamedPipeConnector =
        unsafe { CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTOR, None, CLSCTX_LOCAL_SERVER) }
            .expect("CoCreateInstance(CLSCTX_LOCAL_SERVER) for the connector");
    assert_eq!(server_pids(server), vec![pid], "the second class started a second server");

    let client = connector
        .ConnectAsync(&HSTRING::from(&name))
        .expect("ConnectAsync")
        .join()
        .expect("connect");
    let accepted = accepted_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("ConnectionReceived never reached this process")
        .resolve()
        .expect("resolve the accepted stream");

    // --- 4. Bytes cross, both ways ----------------------------------------
    // Both ends are proxies: the pipe connects two objects in the server.
    // The buffers are system `Buffer`s, which marshal by value, so the bytes
    // travel with them.
    write(&client, b"ping");
    assert_eq!(read(&accepted, 16), b"ping");
    write(&accepted, b"pong!");
    assert_eq!(read(&client, 16), b"pong!");

    // --- 5. Cancellation crosses ------------------------------------------
    let pending = client
        .cast::<IInputStream>()
        .unwrap()
        .ReadAsync(&Buffer::Create(16).unwrap(), 16, InputStreamOptions::Partial)
        .expect("ReadAsync");
    let info: IAsyncInfo = pending.cast().unwrap();
    info.Cancel().expect("Cancel");
    assert!(pending.join().is_err(), "a cancelled read reported success");
    assert_eq!(info.Status().unwrap(), AsyncStatus::Canceled);

    // --- 6. A buffer that cannot carry its bytes is refused clearly -------
    // A Rust `IBuffer` with byte access, which works in-process. Across the
    // boundary it travels as a proxy, `IBufferByteAccess` does not travel at
    // all, and the server must say so rather than fail obscurely.
    let local: IBuffer = LocalBuffer(core::cell::UnsafeCell::new([0u8; 8]), 0.into()).into();
    let err = client
        .cast::<IOutputStream>()
        .unwrap()
        .WriteAsync(&local)
        .and_then(|op| op.join())
        .expect_err("a non-marshalling buffer was accepted out of process");
    assert_eq!(err.code(), E_INVALIDARG, "{err:?}");

    // --- 7. Close crosses, and everything is released ---------------------
    client.cast::<IClosable>().unwrap().Close().expect("Close client");
    accepted.cast::<IClosable>().unwrap().Close().expect("Close accepted");
    listener.RemoveConnectionReceived(token).expect("remove ConnectionReceived");
    listener.cast::<IClosable>().unwrap().Close().expect("Close listener");
    drop((client, accepted, info, pending, connector, listener));
    pid
}

/// A server that dies under a client is an error the client gets back, not a
/// crash in the client.
fn server_death_is_an_error_not_a_crash(server: &Path) {
    let connector: ITransport =
        unsafe { CoCreateInstance(&CLSID_NAMED_PIPE_CONNECTOR, None, CLSCTX_LOCAL_SERVER) }
            .expect("CoCreateInstance for the second server");
    let pids = server_pids(server);
    assert_eq!(pids.len(), 1, "expected a fresh server, found {pids:?}");

    let process = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pids[0]) }
        .expect("OpenProcess");
    unsafe { TerminateProcess(process, 1) }.expect("TerminateProcess");
    assert_eq!(unsafe { WaitForSingleObject(process, 5000) }, WAIT_OBJECT_0);
    let _ = unsafe { CloseHandle(process) };

    let err = connector.Transport().expect_err("a call into a dead server succeeded");
    // Which one depends on how far COM had got in noticing; all three mean
    // "the server is gone".
    const RPC_E_DISCONNECTED: HRESULT = HRESULT(0x80010108u32 as i32);
    const RPC_S_SERVER_UNAVAILABLE: HRESULT = HRESULT(0x800706BAu32 as i32);
    const RPC_S_CALL_FAILED: HRESULT = HRESULT(0x800706BEu32 as i32);
    assert!(
        [RPC_E_DISCONNECTED, RPC_S_SERVER_UNAVAILABLE, RPC_S_CALL_FAILED].contains(&err.code()),
        "unexpected error from a dead server: {err:?}"
    );
}

/// A minimal in-process `IBuffer`: everything a local caller needs, and
/// nothing that lets it marshal by value.
#[implement(IBuffer, IBufferByteAccess)]
struct LocalBuffer(core::cell::UnsafeCell<[u8; 8]>, core::sync::atomic::AtomicU32);

impl IBuffer_Impl for LocalBuffer_Impl {
    fn Capacity(&self) -> windows_core::Result<u32> {
        Ok(8)
    }
    fn Length(&self) -> windows_core::Result<u32> {
        Ok(self.1.load(core::sync::atomic::Ordering::Relaxed))
    }
    fn SetLength(&self, value: u32) -> windows_core::Result<()> {
        self.1.store(value.min(8), core::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

impl IBufferByteAccess_Impl for LocalBuffer_Impl {
    fn Buffer(&self) -> windows_core::Result<*mut u8> {
        Ok(self.0.get().cast())
    }
}

fn write(stream: &ITransportStream, bytes: &[u8]) {
    let buffer = Buffer::Create(bytes.len() as u32).unwrap();
    let access: IBufferByteAccess = buffer.cast().unwrap();
    unsafe {
        access
            .Buffer()
            .unwrap()
            .copy_from_nonoverlapping(bytes.as_ptr(), bytes.len())
    };
    buffer.SetLength(bytes.len() as u32).unwrap();
    let written = stream
        .cast::<IOutputStream>()
        .unwrap()
        .WriteAsync(&buffer.cast::<IBuffer>().unwrap())
        .expect("WriteAsync")
        .join()
        .expect("write");
    assert_eq!(written as usize, bytes.len());
}

/// Reads once. The bytes come from the operation's *result* buffer: a buffer
/// that crossed by value is a copy, so the one passed in is never written.
fn read(stream: &ITransportStream, capacity: u32) -> Vec<u8> {
    let result = stream
        .cast::<IInputStream>()
        .unwrap()
        .ReadAsync(&Buffer::Create(capacity).unwrap(), capacity, InputStreamOptions::Partial)
        .expect("ReadAsync")
        .join()
        .expect("read");
    let len = result.Length().unwrap() as usize;
    let access: IBufferByteAccess = result.cast().expect("result buffer has byte access here");
    unsafe { std::slice::from_raw_parts(access.Buffer().unwrap(), len) }.to_vec()
}

/// Builds the server and the proxy/stub, which nothing this test links
/// depends on; see `tests/activation.rs`'s `ensure_dll` for why a nested
/// build is safe. Returns the profile directory both land in.
fn build() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target dir")
        .to_path_buf();
    let mut cmd = std::process::Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "winasync-server", "-p", "winasync-ps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if dir.file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    let out = cmd.output().expect("spawn cargo build");
    assert!(
        out.status.success(),
        "building the server and proxy failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// The keys an installer of the out-of-process server writes.
struct Registered(Vec<String>);

impl Registered {
    fn install(server: &Path, proxy: &Path) -> Self {
        let mut written = Vec::new();
        // Quoted: COM splits an unquoted path at the first space, and appends
        // `-Embedding` to whatever it ends up with.
        let launch = format!("\"{}\"", server.display());
        for class in CLASSES {
            let base = format!("Software\\Classes\\CLSID\\{}", clsid_key(&class.clsid));
            unsafe {
                let key = create_key(&base);
                set_sz(key, PCWSTR::null(), class.description);
                let _ = RegCloseKey(key);
                let key = create_key(&format!("{base}\\LocalServer32"));
                set_sz(key, PCWSTR::null(), &launch);
                let _ = RegCloseKey(key);
            }
            written.push(base);
        }
        for iid in PROXIED_IIDS {
            let base = format!("Software\\Classes\\Interface\\{}", clsid_key(iid));
            unsafe {
                let key = create_key(&format!("{base}\\ProxyStubClsid32"));
                set_sz(key, PCWSTR::null(), &clsid_key(&CLSID_PROXY_STUB));
                let _ = RegCloseKey(key);
            }
            written.push(base);
        }
        let base = format!("Software\\Classes\\CLSID\\{}", clsid_key(&CLSID_PROXY_STUB));
        unsafe {
            let key = create_key(&format!("{base}\\InprocServer32"));
            set_sz(key, PCWSTR::null(), &proxy.display().to_string());
            set_sz(key, PCWSTR(windows_core::w!("ThreadingModel").as_ptr()), "Both");
            let _ = RegCloseKey(key);
        }
        written.push(base);
        Self(written)
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        for path in &self.0 {
            let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(path)) };
            if status.is_err() {
                eprintln!("removing {path} failed: {status:?}; HKCU keys may remain");
            }
        }
    }
}

/// # Safety
///
/// The returned key is open and must be closed by the caller.
unsafe fn create_key(path: &str) -> HKEY {
    let mut key = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
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
    assert!(status.is_ok(), "RegCreateKeyExW({path}): {status:?}");
    key
}

/// # Safety
///
/// `key` must be open for writing; `name` null or a valid wide string.
unsafe fn set_sz(key: HKEY, name: PCWSTR, value: &str) {
    let mut bytes = Vec::with_capacity((value.len() + 1) * 2);
    for u in value.encode_utf16().chain(core::iter::once(0u16)) {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let status = unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(&bytes)) };
    assert!(status.is_ok(), "RegSetValueExW: {status:?}");
}

/// Running processes whose image is exactly `exe`. COM starts the server, so
/// the test has no handle of its own and finds it by path -- the full path,
/// so a server from another checkout or profile is not mistaken for ours.
fn server_pids(exe: &Path) -> Vec<u32> {
    let want = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let mut found = Vec::new();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.expect("snapshot");
    let mut entry = PROCESSENTRY32W {
        dwSize: core::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(SERVER_EXE)
            && image_path(entry.th32ProcessID)
                .and_then(|p| std::fs::canonicalize(p).ok())
                .is_some_and(|p| p == want)
        {
            found.push(entry.th32ProcessID);
        }
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    found
}

fn image_path(pid: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows_core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    let _ = unsafe { CloseHandle(process) };
    ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn wait_for_exit(pid: u32, when: &str) {
    let Ok(process): Result<HANDLE, _> = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) })
    else {
        return; // already gone
    };
    let start = Instant::now();
    let waited = unsafe { WaitForSingleObject(process, EXIT_WITHIN.as_millis() as u32) };
    let _ = unsafe { CloseHandle(process) };
    assert_eq!(
        waited, WAIT_OBJECT_0,
        "the server did not exit {when} (waited {:?})",
        start.elapsed()
    );
}
