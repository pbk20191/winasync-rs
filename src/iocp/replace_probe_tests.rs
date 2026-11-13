//! Probe: can a handle's completion port be replaced or removed after the
//! fact?
//!
//! `CreateIoCompletionPort` refuses a handle that already has a port
//! (`ERROR_INVALID_PARAMETER`), which is the rule behind
//! `TcpListener::accept_into` keeping a recycled socket's registration and
//! behind `ioring` refusing a port-bound handle. `NtSetInformationFile` with
//! `FileReplaceCompletionInformation` (Windows 8.1+) claims to replace the
//! port, or remove it with a null port. This measures what it actually does
//! for a socket, a file, and a handle a thread-pool `TP_IO` owns.
//!
//! Run by hand: `cargo test --features iocp replace_completion_port -- --ignored --nocapture`.

use std::net::{TcpListener, TcpStream};
use std::os::windows::io::AsRawSocket;
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows::Win32::Networking::WinSock::{send, SEND_RECV_FLAGS, SOCKET, WSABUF, WSARecv};
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, IO_STATUS_BLOCK, OVERLAPPED,
};

#[repr(C)]
struct FileCompletionInformation {
    port: HANDLE,
    key: *mut core::ffi::c_void,
}

const FILE_REPLACE_COMPLETION_INFORMATION: i32 = 61;

type NtSetInformationFile = unsafe extern "system" fn(
    HANDLE,
    *mut IO_STATUS_BLOCK,
    *const core::ffi::c_void,
    u32,
    i32,
) -> i32;

fn nt_set_information_file() -> NtSetInformationFile {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    unsafe {
        let ntdll = GetModuleHandleW(windows_core::w!("ntdll.dll")).expect("ntdll");
        let p = GetProcAddress(ntdll, windows_core::PCSTR(c"NtSetInformationFile".as_ptr().cast()))
            .expect("NtSetInformationFile");
        core::mem::transmute::<unsafe extern "system" fn() -> isize, NtSetInformationFile>(p)
    }
}

/// `FileReplaceCompletionInformation`: `port` null removes the association.
fn replace(handle: HANDLE, port: HANDLE, key: usize) -> i32 {
    let info = FileCompletionInformation {
        port,
        key: key as *mut core::ffi::c_void,
    };
    let mut iosb = IO_STATUS_BLOCK::default();
    unsafe {
        nt_set_information_file()(
            handle,
            &mut iosb,
            &info as *const _ as *const core::ffi::c_void,
            size_of::<FileCompletionInformation>() as u32,
            FILE_REPLACE_COMPLETION_INFORMATION,
        )
    }
}

fn new_port() -> HANDLE {
    unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 0) }.expect("port")
}

fn associate(handle: HANDLE, port: HANDLE, key: usize) -> windows_core::Result<()> {
    unsafe { CreateIoCompletionPort(handle, Some(port), key, 0) }.map(|_| ())
}

/// One dequeue with a timeout: `Some(key)` or `None` on timeout.
fn dequeue(port: HANDLE, timeout: Duration) -> Option<usize> {
    let mut bytes = 0u32;
    let mut key = 0usize;
    let mut ovl: *mut OVERLAPPED = core::ptr::null_mut();
    let r = unsafe {
        GetQueuedCompletionStatus(port, &mut bytes, &mut key, &mut ovl, timeout.as_millis() as u32)
    };
    match r {
        Ok(()) => Some(key),
        Err(e) if e.code() == windows::Win32::Foundation::WIN32_ERROR(WAIT_TIMEOUT.0).to_hresult() => None,
        // A failed I/O still dequeues with its key; only a null OVERLAPPED is
        // a dequeue failure.
        Err(_) if !ovl.is_null() => Some(key),
        Err(e) => panic!("GetQueuedCompletionStatus: {e:?}"),
    }
}

/// A connected loopback pair: (ours, peer). `std` creates sockets
/// overlapped, which is what a completion port needs.
fn pair() -> (TcpStream, TcpStream) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let ours = TcpStream::connect(addr).unwrap();
    let (peer, _) = l.accept().unwrap();
    (ours, peer)
}

/// Posts a one-byte receive, has the peer send one byte, and returns the
/// OVERLAPPED so it outlives the completion.
fn recv_then_send(ours: &TcpStream, peer: &TcpStream, buf: &mut [u8; 1]) -> Box<OVERLAPPED> {
    let mut ovl = Box::new(OVERLAPPED::default());
    let wsabuf = WSABUF {
        len: 1,
        buf: windows_core::PSTR(buf.as_mut_ptr()),
    };
    let mut flags = 0u32;
    let r = unsafe {
        WSARecv(
            SOCKET(ours.as_raw_socket() as usize),
            &[wsabuf],
            None,
            &mut flags,
            Some(&mut *ovl),
            None,
        )
    };
    // 0 or WSA_IO_PENDING both mean a completion will follow.
    let _ = r;
    let n = unsafe { send(SOCKET(peer.as_raw_socket() as usize), b"x", SEND_RECV_FLAGS(0)) };
    assert_eq!(n, 1);
    ovl
}

fn sock_handle(s: &TcpStream) -> HANDLE {
    HANDLE(s.as_raw_socket() as usize as *mut core::ffi::c_void)
}

#[test]
#[ignore = "probe, prints its findings; run with --ignored --nocapture"]
fn replace_completion_port() {
    let short = Duration::from_millis(300);
    // Printed as they happen: a fast-fail in a later step must not hide the
    // earlier findings.
    fn say(line: String) {
        println!("{line}");
    }

    // --- A socket --------------------------------------------------------
    {
        let (ours, peer) = pair();
        let h = sock_handle(&ours);
        let a = new_port();
        let b = new_port();
        let c = new_port();
        associate(h, a, 1).expect("first association");
        let mut buf = [0u8; 1];
        let _o1 = recv_then_send(&ours, &peer, &mut buf);
        say(format!("socket: first completion on A -> key {:?}", dequeue(a, short)));

        // The documented refusal.
        say(format!(
            "socket: CreateIoCompletionPort to B while bound to A -> {:?}",
            associate(h, b, 2).err().map(|e| e.code())
        ));

        // Replace A with B.
        let st = replace(h, b, 2);
        let _o2 = recv_then_send(&ours, &peer, &mut buf);
        say(format!(
            "socket: replace A->B status 0x{:08x}; then completion on B -> {:?}, on A -> {:?}",
            st as u32,
            dequeue(b, short),
            dequeue(a, short)
        ));

        // Remove, then associate the ordinary way.
        let st = replace(h, HANDLE::default(), 0);
        let re = associate(h, c, 3);
        let _o3 = recv_then_send(&ours, &peer, &mut buf);
        say(format!(
            "socket: remove status 0x{:08x}; CreateIoCompletionPort to C -> {:?}; completion on C -> {:?}, on B -> {:?}",
            st as u32,
            re.err().map(|e| e.code()),
            dequeue(c, short),
            dequeue(b, short)
        ));

        // Removed and left unbound: does the receive still work, synchronously
        // or via the OVERLAPPED event?
        let st = replace(h, HANDLE::default(), 0);
        let mut ovl = Box::new(OVERLAPPED::default());
        let ev = unsafe {
            windows::Win32::System::Threading::CreateEventW(None, true, false, None).unwrap()
        };
        ovl.hEvent = ev;
        let wsabuf = WSABUF {
            len: 1,
            buf: windows_core::PSTR(buf.as_mut_ptr()),
        };
        let mut flags = 0u32;
        let r = unsafe {
            WSARecv(
                SOCKET(ours.as_raw_socket() as usize),
                &[wsabuf],
                None,
                &mut flags,
                Some(&mut *ovl),
                None,
            )
        };
        let n = unsafe { send(SOCKET(peer.as_raw_socket() as usize), b"x", SEND_RECV_FLAGS(0)) };
        assert_eq!(n, 1);
        let w = unsafe { windows::Win32::System::Threading::WaitForSingleObject(ev, 1000) };
        say(format!(
            "socket: remove again status 0x{:08x}; unbound WSARecv -> {:?}; event wait -> {:?}; stray completion on C -> {:?}",
            st as u32,
            r,
            w,
            dequeue(c, short)
        ));
        unsafe {
            let _ = CloseHandle(ev);
            let _ = CloseHandle(a);
            let _ = CloseHandle(b);
            let _ = CloseHandle(c);
        }
    }

    // --- A handle owned by a thread-pool TP_IO -----------------------------
    {
        use windows::Win32::System::Threading::{
            CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
            WaitForThreadpoolIoCallbacks, PTP_CALLBACK_INSTANCE, PTP_IO,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLBACKS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "system" fn cb(
            _: PTP_CALLBACK_INSTANCE,
            _: *mut core::ffi::c_void,
            _: *mut core::ffi::c_void,
            _: u32,
            _: usize,
            _: PTP_IO,
        ) {
            CALLBACKS.fetch_add(1, Ordering::SeqCst);
        }

        let (ours, peer) = pair();
        let h = sock_handle(&ours);
        let io = unsafe { CreateThreadpoolIo(h, Some(cb), None, None) }.expect("TP_IO");
        let mut buf = [0u8; 1];
        unsafe { StartThreadpoolIo(io) };
        let _o1 = recv_then_send(&ours, &peer, &mut buf);
        unsafe { WaitForThreadpoolIoCallbacks(io, false) };
        say(format!("tp_io: callbacks after first op -> {}", CALLBACKS.load(Ordering::SeqCst)));

        // Take the handle away from the pool's port and give it to ours.
        let st = replace(h, HANDLE::default(), 0);
        let d = new_port();
        let re = associate(h, d, 4);
        let _o2 = recv_then_send(&ours, &peer, &mut buf);
        say(format!(
            "tp_io: remove status 0x{:08x}; CreateIoCompletionPort to D -> {:?}; completion on D -> {:?}; callbacks -> {}",
            st as u32,
            re.err().map(|e| e.code()),
            dequeue(d, short),
            CALLBACKS.load(Ordering::SeqCst)
        ));

        // And back: remove from D, bind a fresh TP_IO.
        let st = replace(h, HANDLE::default(), 0);
        unsafe { CloseThreadpoolIo(io) };
        let io2 = unsafe { CreateThreadpoolIo(h, Some(cb), None, None) };
        match io2 {
            Ok(io2) => {
                unsafe { StartThreadpoolIo(io2) };
                let _o3 = recv_then_send(&ours, &peer, &mut buf);
                unsafe { WaitForThreadpoolIoCallbacks(io2, false) };
                say(format!(
                    "tp_io: remove status 0x{:08x}; second CreateThreadpoolIo ok; callbacks -> {}; stray on D -> {:?}",
                    st as u32,
                    CALLBACKS.load(Ordering::SeqCst),
                    dequeue(d, short)
                ));
                // No `CancelThreadpoolIo`: the one `Start` was consumed by
                // the callback, and cancelling with nothing started is a
                // fail-fast (`STATUS_INVALID_PARAMETER`), measured here.
                unsafe { CloseThreadpoolIo(io2) };
            }
            Err(e) => say(format!(
                "tp_io: remove status 0x{:08x}; second CreateThreadpoolIo -> {:?}",
                st as u32,
                e.code()
            )),
        }
        unsafe {
            let _ = CloseHandle(d);
        }
    }

    // --- A file ------------------------------------------------------------
    {
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, ReadFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        };
        let path = std::env::temp_dir().join(format!("replace_probe_{}.bin", std::process::id()));
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        let h = unsafe {
            CreateFileW(
                &windows_core::HSTRING::from(path.to_string_lossy().as_ref()),
                windows::Win32::Foundation::GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .unwrap();
        let a = new_port();
        let b = new_port();
        associate(h, a, 5).unwrap();
        let read = |h: HANDLE| {
            let mut ovl = Box::new(OVERLAPPED::default());
            let mut buf = vec![0u8; 4096];
            let r = unsafe { ReadFile(h, Some(&mut buf[..]), None, Some(&mut *ovl)) };
            (ovl, buf, r.err().map(|e| e.code()))
        };
        let (_o1, _b1, r1) = read(h);
        say(format!("file: issue -> {:?}; completion on A -> {:?}", r1, dequeue(a, short)));
        let st = replace(h, b, 6);
        let (_o2, _b2, _) = read(h);
        say(format!(
            "file: replace A->B status 0x{:08x}; completion on B -> {:?}, on A -> {:?}",
            st as u32,
            dequeue(b, short),
            dequeue(a, short)
        ));
        unsafe {
            let _ = CloseHandle(h);
            let _ = CloseHandle(a);
            let _ = CloseHandle(b);
        }
        let _ = std::fs::remove_file(&path);
    }
}
