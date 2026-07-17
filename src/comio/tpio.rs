//! Named pipe / AF_UNIX WinRT streams via Win32 overlapped IO.
//!
//! TP_IO overlapped engine mirroring `tp::io`'s op-block discipline, but the
//! completion action is a caller-supplied boxed closure (later: fill an
//! IBuffer + fire a COM completion handler) rather than a Rust waker, and
//! cancellation is exposed via [`OpHandle`] instead of `Future::drop`.

use core::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{closesocket, SOCKET};
use windows::Win32::System::Threading::{
    CancelThreadpoolIo, CloseThreadpoolIo, CreateThreadpoolIo, StartThreadpoolIo,
    PTP_CALLBACK_INSTANCE, PTP_IO,
};
use windows::Win32::System::IO::{CancelIoEx, OVERLAPPED};

/// Number of live operation blocks (debug builds) — lets tests assert that
/// cancel storms don't leak.
#[cfg(debug_assertions)]
pub(crate) static LIVE_OPS: AtomicUsize = AtomicUsize::new(0);

/// A handle registered with the pool's TP_IO. Owns both the kernel handle and
/// the TP_IO object.
pub(crate) struct IoReg {
    pub(crate) handle: HANDLE,
    io: PTP_IO,
    is_socket: bool,
}

unsafe impl Send for IoReg {}
unsafe impl Sync for IoReg {}

impl IoReg {
    /// Takes ownership of `handle` on success; on error the caller still owns
    /// (and must close) it.
    pub(crate) fn register(handle: HANDLE, is_socket: bool) -> windows_core::Result<Arc<Self>> {
        let io = unsafe {
            // Default thread-pool callback environment (pcbe = NULL); no
            // per-object context, the per-op OVERLAPPED pointer identifies
            // the operation.
            CreateThreadpoolIo(handle, Some(io_complete), None, None)?
        };
        Ok(Arc::new(Self {
            handle,
            io,
            is_socket,
        }))
    }

    fn socket(&self) -> SOCKET {
        SOCKET(self.handle.0 as usize)
    }
}

impl Drop for IoReg {
    fn drop(&mut self) {
        // Every in-flight op holds an Arc<IoReg> that the completion callback
        // releases only at its tail, so at this point every started IO has
        // had its completion delivered — closing immediately is legal. This
        // may run at the tail of the last completion callback itself, which
        // is exactly why WaitForThreadpoolIoCallbacks is NOT used here (it
        // would self-deadlock).
        unsafe {
            CloseThreadpoolIo(self.io);
            if self.is_socket {
                closesocket(self.socket());
            } else {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

/// Outcome of issuing the overlapped call, as classified by the caller.
pub(crate) enum Issue {
    /// A completion callback will arrive.
    Pending,
    /// Hard synchronous failure — no completion will arrive.
    Failed(windows_core::HRESULT),
}

type CompleteFn = Box<dyn FnOnce(u32, usize) + Send>;

struct OpShared {
    fired: bool,
    on_complete: Option<CompleteFn>,
}

struct OpHeader {
    /// 2 at issue time: one for the caller (`OpHandle`), one for the
    /// kernel/completion side.
    refs: AtomicUsize,
    shared: Mutex<OpShared>,
    /// Keeps the handle and TP_IO alive while the op is in flight.
    reg: Arc<IoReg>,
}

/// Heap-allocated operation block. `repr(C)` with the OVERLAPPED first, so the
/// completion callback's `overlapped` pointer *is* the block pointer.
#[repr(C)]
struct Op {
    overlapped: OVERLAPPED,
    header: OpHeader,
}

unsafe fn release_ref(op: *mut c_void) {
    let header = unsafe { &(*(op as *mut Op)).header };
    if header.refs.fetch_sub(1, Ordering::Release) == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        #[cfg(debug_assertions)]
        LIVE_OPS.fetch_sub(1, Ordering::Relaxed);
        unsafe { drop(Box::from_raw(op as *mut Op)) };
    }
}

/// The single completion callback for every registered handle.
unsafe extern "system" fn io_complete(
    _: PTP_CALLBACK_INSTANCE,
    _ctx: *mut c_void,
    overlapped: *mut c_void,
    ioresult: u32,
    bytes: usize,
    _io: PTP_IO,
) {
    let header = unsafe { &(*(overlapped as *mut Op)).header };
    let cb = {
        let mut s = header.shared.lock().unwrap();
        s.fired = true;
        s.on_complete.take()
    };
    if let Some(cb) = cb {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || cb(ioresult, bytes)));
    }
    // Tail: release the kernel-side reference (may free the block and, if it
    // held the last Arc<IoReg>, close the handle).
    unsafe { release_ref(overlapped) };
}

/// Opaque handle to an in-flight operation, identifying it for cancellation.
/// Holds the caller-side ref; dropping it releases that ref.
pub(crate) struct OpHandle {
    op: *mut Op,
}

unsafe impl Send for OpHandle {}

impl OpHandle {
    pub(crate) fn cancel(&self) {
        let header = unsafe { &(*self.op).header };
        let in_flight = !header.shared.lock().unwrap().fired;
        if in_flight {
            unsafe {
                let _ = CancelIoEx(header.reg.handle, Some(&(*self.op).overlapped));
            }
        }
    }
}

impl Drop for OpHandle {
    fn drop(&mut self) {
        unsafe { release_ref(self.op as *mut c_void) }
    }
}

/// Allocates an op block, arms the TP_IO (`StartThreadpoolIo` *before*
/// issuing), runs `issue`, and on `Failed` balances with
/// `CancelThreadpoolIo` and calls `on_complete` synchronously.
///
/// # Safety
/// `issue` must issue exactly one overlapped operation on `reg.handle` using
/// the provided OVERLAPPED pointer and classify its outcome truthfully:
/// reporting `Failed` when a completion is still coming leads to
/// use-after-free; reporting `Pending` when none is coming leaks the block
/// and permanently unbalances the TP_IO.
pub(crate) unsafe fn submit(
    reg: &Arc<IoReg>,
    on_complete: CompleteFn,
    issue: impl FnOnce(*mut OVERLAPPED) -> Issue,
) -> OpHandle {
    let op = Box::into_raw(Box::new(Op {
        overlapped: unsafe { mem::zeroed() },
        header: OpHeader {
            refs: AtomicUsize::new(2),
            shared: Mutex::new(OpShared {
                fired: false,
                on_complete: Some(on_complete),
            }),
            reg: reg.clone(),
        },
    }));
    #[cfg(debug_assertions)]
    LIVE_OPS.fetch_add(1, Ordering::Relaxed);

    unsafe { StartThreadpoolIo(reg.io) };
    let outcome = unsafe { issue(&raw mut (*op).overlapped) };
    if let Issue::Failed(hr) = outcome {
        unsafe {
            CancelThreadpoolIo(reg.io);
            let cb = {
                let mut s = (*op).header.shared.lock().unwrap();
                s.fired = true;
                s.on_complete.take()
            };
            if let Some(cb) = cb {
                // Map the HRESULT to a nonzero Win32 error code: `windows`
                // 0.62.2's `WIN32_ERROR` has no `from_error`/`from_hresult`
                // conversion, so extract the low 16 bits when the HRESULT is
                // in the FACILITY_WIN32 form (HRESULT_FROM_WIN32 layout),
                // else fall back to a generic nonzero code (ERROR_GEN_FAILURE
                // = 31). Only "nonzero" is load-bearing for callers.
                let hr_val = hr.0 as u32;
                let win32 = if (hr_val >> 16) & 0x7ff == 7 {
                    hr_val & 0xffff
                } else {
                    31
                };
                // Mirror io_complete: a panic in `cb` must not skip
                // release_ref below, or the Op block (and its Arc<IoReg>)
                // leaks permanently.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    cb(win32, 0)
                }));
            }
            release_ref(op as *mut c_void);
        }
    }
    OpHandle { op }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use windows::Win32::Foundation::ERROR_IO_PENDING;
    use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, CreateFileW, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::CreateNamedPipeW;
    use windows::Win32::System::Pipes::{PIPE_TYPE_BYTE, PIPE_WAIT};

    // Minimal overlapped write-then-read over a named pipe pair.
    #[test]
    fn overlapped_pipe_roundtrip() {
        let name = format!(r"\\.\pipe\comio_tpio_{}", std::process::id());
        let wname = windows_core::HSTRING::from(&name);
        let server = unsafe {
            CreateNamedPipeW(&wname, windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_OVERLAPPED.0), PIPE_TYPE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, None)
        };
        assert!(!server.is_invalid(), "CreateNamedPipeW failed: {:?}", unsafe { windows::Win32::Foundation::GetLastError() });
        // Client opens the pipe (synchronous connect is fine for the test).
        let client = unsafe {
            CreateFileW(&wname, FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, None)
        }.unwrap();

        let sreg = IoReg::register(server, false).unwrap();
        let creg = IoReg::register(client, false).unwrap();

        // Server writes "Z" via overlapped IO.
        let (tx, rx) = mpsc::channel();
        let wbuf = Box::new([b'Z']);
        let wptr = wbuf.as_ptr();
        let _wop = unsafe {
            submit(&sreg, Box::new(move |ioresult, bytes| { let _ = &wbuf; tx.send((ioresult, bytes)).unwrap(); }), |ovl| {
                match WriteFile(sreg.handle, Some(std::slice::from_raw_parts(wptr, 1)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (wres, wbytes) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!((wres, wbytes), (0, 1));

        // Client reads it back.
        let (tx2, rx2) = mpsc::channel();
        let rbuf = std::sync::Arc::new(std::sync::Mutex::new([0u8; 1]));
        let rbuf2 = rbuf.clone();
        let rptr = rbuf.lock().unwrap().as_mut_ptr();
        let _rop = unsafe {
            submit(&creg, Box::new(move |ioresult, bytes| { let _ = &rbuf2; tx2.send((ioresult, bytes)).unwrap(); }), |ovl| {
                match ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rptr, 1)), None, Some(ovl)) {
                    Ok(()) => Issue::Pending,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (rres, rbytes) = rx2.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!((rres, rbytes), (0, 1));
        assert_eq!(rbuf.lock().unwrap()[0], b'Z');

        drop(sreg); drop(creg);
        let _ = server; let _ = client; // handles closed by IoReg::Drop
    }

    // Regression test: cancelling an in-flight op and dropping its OpHandle
    // must still deliver a completion (releasing the kernel-side ref) so the
    // Op block and its Arc<IoReg> don't leak. Loops a modest number of times
    // so a leak would show up as a nonzero LIVE_OPS count.
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

            let sreg = IoReg::register(server, false).unwrap();
            let creg = IoReg::register(client, false).unwrap();

            // Start a read on the client with NO corresponding write, so it
            // stays pending; cancel it in flight and drop the handle right
            // away, without waiting for the completion ourselves.
            let rbuf = Box::new([0u8; 1]);
            let rptr = rbuf.as_ptr() as *mut u8;
            let rop = unsafe {
                submit(&creg, Box::new(move |_ioresult, _bytes| { let _ = &rbuf; }), |ovl| {
                    match ReadFile(creg.handle, Some(std::slice::from_raw_parts_mut(rptr, 1)), None, Some(ovl)) {
                        Ok(()) => Issue::Pending,
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                        Err(e) => Issue::Failed(e.code()),
                    }
                })
            };
            rop.cancel();
            drop(rop);

            drop(sreg); drop(creg);
            let _ = server; let _ = client; // handles closed by IoReg::Drop
        }

        // The kernel delivers ERROR_OPERATION_ABORTED completions
        // asynchronously; give the threadpool a brief, bounded window to
        // drain them. LIVE_OPS is process-global (shared with other tests
        // running concurrently), so poll for it to settle rather than
        // asserting immediately — mirrors
        // `src/tp/io/file.rs`'s `dropped_read_future_does_not_leak`.
        #[cfg(debug_assertions)]
        {
            for _ in 0..500 {
                if LIVE_OPS.load(Ordering::Relaxed) == 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert_eq!(
                LIVE_OPS.load(Ordering::Relaxed),
                0,
                "op blocks leaked after cancel"
            );
        }
    }
}
