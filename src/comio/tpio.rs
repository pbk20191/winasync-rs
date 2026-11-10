//! comio's overlapped-IO facade: a thin adapter over the shared core
//! ([`crate::common::overlapped`]) preserving comio's API — `IoReg::register`
//! (default process pool), `submit(reg, on_complete, issue)` with a
//! `FnOnce(u32, usize)` completion and an `OpHandle` whose `cancel` is explicit
//! (no cancel-on-drop). The engine, op-block discipline, and `LIVE_OPS` live in
//! the shared core.

use crate::common::overlapped::{self as ov, Binding, Completion, IoReg as CoreReg};
pub(crate) use crate::common::overlapped::IoHandle;
use std::ops::Deref;
use std::sync::Arc;
use windows::Win32::System::IO::OVERLAPPED;

pub(crate) use crate::common::overlapped::Issue;
// Leak counter reached by comio's tests as `tpio::LIVE_OPS`.
#[allow(unused_imports)]
#[cfg(any(test, feature = "counters"))]
pub(crate) use crate::common::overlapped::LIVE_OPS;

/// A handle registered on the **default process pool** (`pcbe = NULL`). Newtype
/// over the shared [`CoreReg`], registered with comio's pool.
pub(crate) struct IoReg(Arc<CoreReg>);

impl Deref for IoReg {
    type Target = CoreReg;
    fn deref(&self) -> &CoreReg {
        &self.0
    }
}

impl IoReg {
    /// Registers `io` with comio's pool. Takes `io` either way: a failed
    /// registration closes it.
    pub(crate) fn register(io: impl Into<IoHandle>) -> windows_core::Result<Arc<Self>> {
        // The process default pool everywhere but the server DLL, where the
        // environment sets `RaceDll` so a completion arriving after a forced
        // unload is not called into freed code; see `comio::pool`.
        let binding = Binding::Pool(super::pool::pool_ref());
        Ok(Arc::new(IoReg(CoreReg::register(io, binding)?)))
    }
}

/// Opaque in-flight op handle; `cancel` is explicit, drop does not cancel.
pub(crate) type OpHandle = ov::OpHandle<()>;

/// Issues one overlapped op over the shared core. `on_complete` is called once
/// with `(ioresult, bytes)`; a synchronous `Issue::Failed(hr)` is delivered as
/// `(win32, 0)` (the exact HRESULT is downgraded to a nonzero win32 code, as
/// comio has always done).
///
/// # Safety
/// Same contract as [`ov::submit`]: `issue` issues exactly one overlapped op and
/// classifies its outcome truthfully.
pub(crate) unsafe fn submit<F: FnOnce(u32, usize) + Send + 'static>(
    reg: &Arc<IoReg>,
    on_complete: F,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
) -> OpHandle {
    let cb = move |c: Completion| match c {
        Completion::Done(ioresult, bytes) => on_complete(ioresult, bytes),
        Completion::SyncFail(hr) => {
            // Map HRESULT -> nonzero win32 (low 16 bits in FACILITY_WIN32 form,
            // else ERROR_GEN_FAILURE=31). Only "nonzero" is load-bearing.
            let hr_val = hr.0 as u32;
            let win32 = if (hr_val >> 16) & 0x7ff == 7 {
                hr_val & 0xffff
            } else {
                31
            };
            on_complete(win32, 0);
        }
    };
    unsafe { ov::submit(&reg.0, (), cb, |ovl, _| issue(ovl)) }
}

#[cfg(test)]
mod tests {
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
            assert_eq!(LIVE_OPS.load(std::sync::atomic::Ordering::Relaxed), 0, "op blocks leaked after cancel");
        }
    }
}
