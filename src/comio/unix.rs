//! AF_UNIX (Unix domain socket) transport: `UnixStreamListener` (`AcceptEx`)
//! and `UnixStreamSocket` (`ConnectEx` fetched via `WSAIoctl`), producing
//! `OverlappedDuplexStream`s (via [`stream::from_reg`]) registered as
//! sockets (`is_socket = true`), so `ReadAsync`/`WriteAsync` drive
//! `WSARecv`/`WSASend`. Mirrors `src/tp/io/net.rs`'s AF_INET
//! `TcpListener`/`TcpStream`, but with `AF_UNIX`/`SOCKADDR_UN` addressing and
//! completion driven through `comio::waitop::await_io` instead of `tp::io`'s
//! own future machinery.

use super::stream;
use super::tpio::{IoReg, Issue};
use super::waitop::await_io;
use core::ffi::c_void;
use std::mem;
use std::sync::{Arc, Once, OnceLock};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Networking::WinSock::{
    bind, closesocket, listen, setsockopt, AcceptEx, ADDRESS_FAMILY, AF_UNIX, INVALID_SOCKET,
    SIO_GET_EXTENSION_FUNCTION_POINTER, SOCKADDR, SOCKADDR_UN, SOCKET, SOCK_STREAM, SOL_SOCKET,
    SOMAXCONN, SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, WSADATA, WSAGetLastError,
    WSAID_CONNECTEX, WSAIoctl, WSASocketW, WSAStartup, WSA_FLAG_OVERLAPPED, WSA_IO_PENDING,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows_core::{Error, IInspectable, Result, HRESULT};

/// Process-wide Winsock initialization; deliberately never torn down
/// (`WSACleanup` is not called). Duplicated from `tp::io::net::wsa_init`
/// (that one is private to `tp`) per the task brief — small, acceptable
/// duplication rather than threading a shared init through crate-visibility
/// changes.
fn wsa_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        let mut data = WSADATA::default();
        let r = WSAStartup(0x0202, &mut data);
        assert_eq!(r, 0, "WSAStartup failed: {r}");
    });
}

fn wsa_last_hr() -> HRESULT {
    WIN32_ERROR(unsafe { WSAGetLastError() }.0 as u32).to_hresult()
}

fn wsa_last_err() -> Error {
    Error::from_hresult(wsa_last_hr())
}

fn sock_check(r: i32) -> Result<()> {
    if r == 0 {
        Ok(())
    } else {
        Err(wsa_last_err())
    }
}

fn new_socket() -> Result<SOCKET> {
    wsa_init();
    unsafe { WSASocketW(AF_UNIX as i32, SOCK_STREAM.0, 0, None, 0, WSA_FLAG_OVERLAPPED) }
}

/// Builds a `SOCKADDR_UN` for `path`. `sun_path` holds the NUL-terminated
/// path bytes (cast `u8` -> `i8`); the path itself must fit in the 108-byte
/// field with room for the trailing NUL, i.e. at most 107 bytes.
fn sockaddr_un(path: &str) -> Result<(SOCKADDR_UN, i32)> {
    let bytes = path.as_bytes();
    if bytes.len() > 107 {
        return Err(Error::from_hresult(
            windows::Win32::Foundation::ERROR_INVALID_PARAMETER.to_hresult(),
        ));
    }
    let mut sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    for (i, &b) in bytes.iter().enumerate() {
        sa.sun_path[i] = b as i8;
    }
    // The trailing NUL is already present (zero-initialized); the address
    // length is the family field plus the path bytes plus that NUL.
    let addrlen = (size_of::<ADDRESS_FAMILY>() + bytes.len() + 1) as i32;
    Ok((sa, addrlen))
}

/// An "autobind" address: family only, no path. `ConnectEx` requires a bound
/// socket (mirrors `tp/io/net.rs` binding to `0.0.0.0:0` before `ConnectEx`);
/// for AF_UNIX the equivalent is binding with a zero-length path.
fn sockaddr_un_autobind() -> (SOCKADDR_UN, i32) {
    let sa = SOCKADDR_UN {
        sun_family: ADDRESS_FAMILY(AF_UNIX),
        sun_path: [0i8; 108],
    };
    (sa, size_of::<ADDRESS_FAMILY>() as i32)
}

fn classify_wsa_overlapped(succeeded: bool) -> Issue {
    if succeeded {
        // Synchronous success still queues a completion (we never set
        // FILE_SKIP_COMPLETION_PORT_ON_SUCCESS).
        Issue::Pending
    } else if unsafe { WSAGetLastError() } == WSA_IO_PENDING {
        Issue::Pending
    } else {
        Issue::Failed(wsa_last_hr())
    }
}

type ConnectExFn = unsafe extern "system" fn(
    s: SOCKET,
    name: *const SOCKADDR,
    namelen: i32,
    lpsendbuffer: *const c_void,
    dwsenddatalength: u32,
    lpdwbytessent: *mut u32,
    lpoverlapped: *mut OVERLAPPED,
) -> windows_core::BOOL;

/// `ConnectEx` is not a direct export: fetch its provider function pointer via
/// `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` once and cache it. Separate
/// cache from `tp::io::net`'s (that one is private, and per-provider pointers
/// are process-stable regardless of which socket fetched them).
fn get_connectex(sock: SOCKET) -> Result<ConnectExFn> {
    static PTR: OnceLock<usize> = OnceLock::new();
    if let Some(&p) = PTR.get() {
        return Ok(unsafe { mem::transmute::<usize, ConnectExFn>(p) });
    }
    let mut fptr: Option<ConnectExFn> = None;
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_CONNECTEX as *const _ as *const c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut fptr as *mut _ as *mut c_void),
            size_of::<Option<ConnectExFn>>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let f = fptr.expect("SIO_GET_EXTENSION_FUNCTION_POINTER returned null ConnectEx");
    let _ = PTR.set(f as usize);
    Ok(f)
}

/// AcceptEx address slot: sockaddr size + 16 bytes, per the API contract.
const ADDR_SLOT: usize = size_of::<SOCKADDR_UN>() + 16;

/// Owns the pre-created accept socket and address buffer while `AcceptEx` is
/// in flight, so both stay alive across the `.await` regardless of how the
/// enclosing `accept()` future is polled. If `accept()`'s own future is
/// dropped before `await_io` resolves, this `Drop` closes the still-owned
/// accept socket (mirrors `tp/io/net.rs`'s `AcceptPayload`); the address
/// buffer is simply freed at that point, same as any other local.
struct AcceptGuard {
    sock: SOCKET,
    _addr_buf: Box<[u8; 2 * ADDR_SLOT]>,
}

impl Drop for AcceptGuard {
    fn drop(&mut self) {
        if self.sock != INVALID_SOCKET {
            unsafe { closesocket(self.sock) };
        }
    }
}

// SOCKET is a plain kernel handle value.
unsafe impl Send for AcceptGuard {}

/// A listening AF_UNIX socket, registered with the thread pool's TP_IO so
/// `AcceptEx` can be driven through [`await_io`].
pub struct UnixStreamListener {
    reg: Arc<IoReg>,
}

impl UnixStreamListener {
    /// Binds a new AF_UNIX socket to `path` and starts listening. AF_UNIX
    /// `bind` fails if a socket file already exists at `path`; callers that
    /// need to rebind a stale path should remove it first (as the test
    /// below does).
    pub fn bind(path: &str) -> Result<Self> {
        let sock = new_socket()?;
        let register = || -> Result<Arc<IoReg>> {
            let (sa, addrlen) = sockaddr_un(path)?;
            sock_check(unsafe { bind(sock, &sa as *const _ as *const SOCKADDR, addrlen) })?;
            sock_check(unsafe { listen(sock, SOMAXCONN as i32) })?;
            IoReg::register(HANDLE(sock.0 as *mut c_void), true)
        };
        match register() {
            Ok(reg) => Ok(UnixStreamListener { reg }),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }

    /// Accepts one connection, returning it as a duplex WinRT stream.
    pub async fn accept(&self) -> Result<IInspectable> {
        let accept_sock = new_socket()?;
        let listen_sock = SOCKET(self.reg.handle.0 as usize);
        let mut guard = AcceptGuard {
            sock: accept_sock,
            _addr_buf: Box::new([0u8; 2 * ADDR_SLOT]),
        };
        let addr_ptr = guard._addr_buf.as_mut_ptr();

        let result = await_io(&self.reg, |ovl| unsafe {
            let mut bytes = 0u32;
            let ok = AcceptEx(
                listen_sock,
                accept_sock,
                addr_ptr as *mut c_void,
                0, // complete on connect; do not wait for first data
                ADDR_SLOT as u32,
                ADDR_SLOT as u32,
                &mut bytes,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;

        // Success: take the socket out of the guard so its `Drop` no longer
        // closes it (`IoReg::register` owns it from here).
        let sock = mem::replace(&mut guard.sock, INVALID_SOCKET);
        let finish = || -> Result<IInspectable> {
            // Until `SO_UPDATE_ACCEPT_CONTEXT` (value = the listen socket)
            // the accepted socket is half-configured.
            let listen_val = listen_sock.0.to_ne_bytes();
            sock_check(unsafe {
                setsockopt(sock, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT, Some(&listen_val))
            })?;
            let reg = IoReg::register(HANDLE(sock.0 as *mut c_void), true)?;
            Ok(stream::from_reg(reg))
        };
        match finish() {
            Ok(s) => Ok(s),
            Err(e) => {
                unsafe { closesocket(sock) };
                Err(e)
            }
        }
    }
}

/// Client side of an AF_UNIX connection.
pub struct UnixStreamSocket;

impl UnixStreamSocket {
    /// Connects to the AF_UNIX socket bound at `path`, returning the
    /// connection as a duplex WinRT stream.
    pub async fn connect(path: &str) -> Result<IInspectable> {
        let sock = new_socket()?;
        let prepared = (|| -> Result<(ConnectExFn, Arc<IoReg>)> {
            // ConnectEx fails with WSAEINVAL on an unbound socket.
            let (local, local_len) = sockaddr_un_autobind();
            sock_check(unsafe { bind(sock, &local as *const _ as *const SOCKADDR, local_len) })?;
            let connect_ex = get_connectex(sock)?;
            let reg = IoReg::register(HANDLE(sock.0 as *mut c_void), true)?;
            Ok((connect_ex, reg))
        })();
        let (connect_ex, reg) = match prepared {
            Ok(v) => v,
            Err(e) => {
                unsafe { closesocket(sock) };
                return Err(e);
            }
        };

        let (remote, remote_len) = match sockaddr_un(path) {
            Ok(v) => v,
            Err(e) => return Err(e), // `reg`'s Drop closes `sock`.
        };
        let result = await_io(&reg, |ovl| unsafe {
            let mut sent = 0u32;
            let ok = connect_ex(
                sock,
                &remote as *const _ as *const SOCKADDR,
                remote_len,
                core::ptr::null(),
                0,
                &mut sent,
                ovl,
            );
            classify_wsa_overlapped(ok.as_bool())
        })
        .await;
        result?;
        sock_check(unsafe { setsockopt(sock, SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT, None) })?;
        Ok(stream::from_reg(reg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::{buffer_ptr, new_buffer};
    use crate::tp::block_on;
    use std::future::IntoFuture;
    use windows::Storage::Streams::{IBuffer, IInputStream, IOutputStream, InputStreamOptions};
    use windows_core::Interface;

    #[test]
    fn af_unix_echo() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("comio_unix_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let sp = p.clone();
        let server = std::thread::spawn(move || {
            let l = UnixStreamListener::bind(&sp).unwrap();
            let s = block_on(l.accept()).unwrap();
            let input: IInputStream = s.cast().unwrap();
            let output: IOutputStream = s.cast().unwrap();
            let rbuf = new_buffer(16).unwrap();
            let got: IBuffer = block_on(
                input
                    .ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 16, InputStreamOptions(0))
                    .unwrap()
                    .into_future(),
            )
            .unwrap();
            block_on(output.WriteAsync(&got).unwrap().into_future()).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let c = block_on(UnixStreamSocket::connect(&p)).unwrap();
        let c_out: IOutputStream = c.cast().unwrap();
        let c_in: IInputStream = c.cast().unwrap();
        let wbuf = new_buffer(4).unwrap();
        unsafe {
            buffer_ptr(&wbuf.clone().into())
                .unwrap()
                .copy_from_nonoverlapping(b"ping".as_ptr(), 4);
        }
        wbuf.SetLength(4).unwrap();
        block_on(
            c_out
                .WriteAsync(&wbuf.cast::<IBuffer>().unwrap())
                .unwrap()
                .into_future(),
        )
        .unwrap();
        let rbuf = new_buffer(4).unwrap();
        let echoed: IBuffer = block_on(
            c_in.ReadAsync(&rbuf.cast::<IBuffer>().unwrap(), 4, InputStreamOptions(0))
                .unwrap()
                .into_future(),
        )
        .unwrap();
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(buffer_ptr(&echoed).unwrap(), 4),
                b"ping"
            );
        }
        server.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }
}
