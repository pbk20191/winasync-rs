use super::*;
use crate::comio::buffer::{buffer_ptr, new_buffer};
use crate::comio::tpio::{IoReg, Issue};
use crate::common::block_on;
use std::future::IntoFuture;
use windows::Storage::Streams::IBuffer;
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};

#[test]
fn async_read_completes() {
    let name = format!(r"\\.\pipe\comio_asyncop_{}", std::process::id());
    let wname = windows_core::HSTRING::from(&name);
    let server = unsafe {
        CreateNamedPipeW(
            &wname,
            FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0),
            PIPE_TYPE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    assert!(!server.is_invalid(), "CreateNamedPipeW failed: {:?}", unsafe {
        windows::Win32::Foundation::GetLastError()
    });
    let client = unsafe {
        CreateFileW(
            &wname,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .unwrap();
    let sreg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(server) }).unwrap();
    let creg = IoReg::register(unsafe { crate::comio::tpio::IoHandle::from_handle(client) }).unwrap();

    // Server: async write "hi".
    let wbuf = new_buffer(2).unwrap();
    let wib: IBuffer = wbuf.clone().into();
    unsafe {
        let p = buffer_ptr(&wib).unwrap();
        p.copy_from_nonoverlapping(b"hi".as_ptr(), 2);
    }
    wbuf.SetLength(2).unwrap();
    let wp = buffer_ptr(&wib).unwrap();
    let sh = sreg.handle;
    let write_op = IoAsyncOp::<u32>::start_write(&sreg, wib.clone(), 2, move |ovl| {
        match unsafe { WriteFile(sh, Some(std::slice::from_raw_parts(wp, 2)), None, Some(ovl)) } {
            Ok(()) => Issue::SyncSuccess,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
            Err(e) => Issue::Failed(e.code()),
        }
    });

    // Client: async read 2 bytes.
    let rbuf = new_buffer(2).unwrap();
    let rib: IBuffer = rbuf.clone().into();
    let rp = buffer_ptr(&rib).unwrap();
    let ch = creg.handle;
    let read_op = IoAsyncOp::start_read(&creg, rib.clone(), (), move |ovl| {
        match unsafe { ReadFile(ch, Some(std::slice::from_raw_parts_mut(rp, 2)), None, Some(ovl)) } {
            Ok(()) => Issue::SyncSuccess,
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
            Err(e) => Issue::Failed(e.code()),
        }
    });

    let wrote = block_on(write_op.into_future()).unwrap();
    assert_eq!(wrote, 2);
    let got = block_on(read_op.into_future()).unwrap();
    assert_eq!(got.Length().unwrap(), 2);
    unsafe {
        assert_eq!(std::slice::from_raw_parts(buffer_ptr(&got).unwrap(), 2), b"hi");
    }
}
