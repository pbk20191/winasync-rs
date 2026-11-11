use super::*;
use std::sync::mpsc;
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::CreateNamedPipeW;
use windows::Win32::System::Pipes::{PIPE_TYPE_BYTE, PIPE_WAIT};

#[test]
fn overlapped_pipe_roundtrip() {
    let name = format!(r"\\.\pipe\comio_tpio_{}", std::process::id());
    let wname = windows_core::HSTRING::from(&name);
    let server = unsafe {
        CreateNamedPipeW(&wname, windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None)
    };
    assert!(!server.is_invalid(), "CreateNamedPipeW failed: {:?}", unsafe { windows::Win32::Foundation::GetLastError() });
    let client = unsafe {
        CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
    }.unwrap();

    let sreg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(server) }).unwrap();
    let creg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(client) }).unwrap();

    let (tx, rx) = mpsc::channel();
    let wbuf = Box::new(*b"Z");
    let wptr = wbuf.as_ptr();
    let _wop = unsafe {
        submit(&sreg, move |ioresult, bytes| { let _ = &wbuf; tx.send((ioresult, bytes)).unwrap(); }, |ovl| {
            match WriteFile(sreg.handle, Some(std::slice::from_raw_parts(wptr, 1)), None, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        })
    };
    let (wres, wbytes) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!((wres, wbytes), (0, 1));

    let (tx2, rx2) = mpsc::channel();
    let rbuf = std::sync::Arc::new(std::sync::Mutex::new([0u8; 1]));
    let rbuf2 = rbuf.clone();
    let rptr = rbuf.lock().unwrap().as_mut_ptr();
    let _rop = unsafe {
        submit(&creg, move |ioresult, bytes| { let _ = &rbuf2; tx2.send((ioresult, bytes)).unwrap(); }, |ovl| {
            match ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rptr, 1)), None, Some(ovl)) {
                Ok(()) => Issue::SyncSuccess,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                Err(e) => Issue::Failed(e.code()),
            }
        })
    };
    let (rres, rbytes) = rx2.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!((rres, rbytes), (0, 1));
    assert_eq!(rbuf.lock().unwrap()[0], b'Z');

    drop(sreg); drop(creg);
    let _ = server; let _ = client;
}

#[test]
fn cancel_in_flight_read_does_not_leak() {
    for i in 0..100 {
        let name = format!(r"\\.\pipe\comio_tpio_cancel_{}_{}", std::process::id(), i);
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe {
            CreateNamedPipeW(&wname, windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None)
        };
        assert!(!server.is_invalid(), "CreateNamedPipeW failed: {:?}", unsafe { windows::Win32::Foundation::GetLastError() });
        let client = unsafe {
            CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
        }.unwrap();

        let sreg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(server) }).unwrap();
        let creg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(client) }).unwrap();

        let rbuf = Box::new([0u8; 1]);
        let rptr = rbuf.as_ptr() as *mut u8;
        let rop = unsafe {
            submit(&creg, Box::new(move |_ioresult, _bytes| { let _ = &rbuf; }), |ovl| {
                match ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rptr, 1)), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        rop.cancel();
        drop(rop);

        drop(sreg); drop(creg);
        let _ = server; let _ = client;
    }

    {
        for _ in 0..500 {
            if LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "{}: {}", "op blocks leaked after cancel", crate::common::overlapped::live_ops_report());
    }
}
