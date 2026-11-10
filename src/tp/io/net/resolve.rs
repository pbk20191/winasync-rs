//! Asynchronous name resolution (`GetAddrInfoExW` with an `OVERLAPPED`).
//!
//! Not an I/O-port operation: the lookup reports completion by signalling
//! the event in its `OVERLAPPED` (there is no completion routine), so the
//! future waits for that event on the runtime's pool through `tp::wait`.
//! Dropping the future cancels the lookup with `GetAddrInfoExCancel` and
//! waits -- on the event, so on no pool thread -- for the lookup to let go
//! of the memory it was given.

use super::{read_sockaddr, SocketOptions, TcpStream};
use crate::tp::Runtime;
use std::net::SocketAddr;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinSock::{
    ADDRINFOEXW, AF_UNSPEC, FreeAddrInfoExW, GetAddrInfoExCancel, GetAddrInfoExOverlappedResult,
    GetAddrInfoExW, IPPROTO_TCP, NS_ALL, SOCK_STREAM, SOCKADDR_STORAGE, WSA_IO_PENDING,
};
use windows::Win32::System::IO::OVERLAPPED;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows_core::{Error, HSTRING, Result};

/// One lookup's memory, which the system writes until the lookup completes.
/// Boxed so its address holds; freed only once the lookup has let go.
struct Lookup {
    ovl: OVERLAPPED,
    result: *mut ADDRINFOEXW,
    cancel: HANDLE,
    event: HANDLE,
    in_flight: bool,
}

// Raw handles and a pointer this lookup owns.
unsafe impl Send for Lookup {}

impl Drop for Lookup {
    fn drop(&mut self) {
        unsafe {
            if self.in_flight {
                // The lookup still completes after a cancel, and signals the
                // event when it has.
                let _ = GetAddrInfoExCancel(&self.cancel);
                WaitForSingleObject(self.event, INFINITE);
            }
            if !self.result.is_null() {
                FreeAddrInfoExW(Some(self.result));
            }
            let _ = CloseHandle(self.event);
        }
    }
}

/// Resolves `host` to its addresses, each with `port`, in the order the
/// system prefers them.
///
/// `host` may be a name or a numeric address. Dropping the future cancels
/// the lookup.
pub fn resolve(
    rt: &Runtime,
    host: &str,
    port: u16,
) -> impl Future<Output = Result<Vec<SocketAddr>>> + Send + use<> {
    let rt = rt.clone();
    let host = HSTRING::from(host);
    let service = HSTRING::from(port.to_string());
    async move {
        crate::common::winsock::wsa_init();
        // Everything holding a raw pointer stays inside this block, so none
        // of it lives across the await below and the future stays `Send`.
        let (mut lookup, issued) = {
            let mut lookup = Box::new(Lookup {
                ovl: OVERLAPPED::default(),
                result: core::ptr::null_mut(),
                cancel: HANDLE::default(),
                event: unsafe { CreateEventW(None, true, false, None)? },
                in_flight: false,
            });
            lookup.ovl.hEvent = lookup.event;
            // One entry per address rather than one per socket type.
            let hints = ADDRINFOEXW {
                ai_family: AF_UNSPEC.0 as i32,
                ai_socktype: SOCK_STREAM.0,
                ai_protocol: IPPROTO_TCP.0,
                ..Default::default()
            };
            let issued = unsafe {
                let l = &mut *lookup;
                GetAddrInfoExW(
                    &host,
                    &service,
                    NS_ALL,
                    None,
                    Some(&hints),
                    &mut l.result,
                    None,
                    Some(&l.ovl),
                    None,
                    Some(&mut l.cancel),
                )
            };
            (lookup, issued)
        };
        match issued {
            0 => {}
            code if code == WSA_IO_PENDING.0 => {
                lookup.in_flight = true;
                // SAFETY: `lookup` owns the event and outlives this wait.
                let wait = unsafe { crate::tp::wait::wait_for_handle(&rt, lookup.event) };
                wait.await;
                lookup.in_flight = false;
                let code = unsafe { GetAddrInfoExOverlappedResult(&lookup.ovl) };
                if code != 0 {
                    return Err(wsa_error(code));
                }
            }
            code => return Err(wsa_error(code)),
        }
        Ok(collect(&lookup))
    }
}

/// The addresses in a completed lookup's result list.
fn collect(lookup: &Lookup) -> Vec<SocketAddr> {
    let mut addrs = Vec::new();
    let mut entry = lookup.result;
    while !entry.is_null() {
        let info = unsafe { &*entry };
        // `ai_addr` is `ai_addrlen` bytes of whichever SOCKADDR the family
        // uses; copied into storage of the size the reader expects.
        let mut storage = SOCKADDR_STORAGE::default();
        let len = info.ai_addrlen.min(size_of::<SOCKADDR_STORAGE>());
        unsafe {
            core::ptr::copy_nonoverlapping(
                info.ai_addr.cast::<u8>(),
                (&mut storage as *mut SOCKADDR_STORAGE).cast::<u8>(),
                len,
            )
        };
        if let Ok(addr) = read_sockaddr(&storage) {
            addrs.push(addr);
        }
        entry = info.ai_next;
    }
    addrs
}

fn wsa_error(code: i32) -> Error {
    Error::from_hresult(windows::Win32::Foundation::WIN32_ERROR(code as u32).to_hresult())
}

impl TcpStream {
    /// Resolves `host` and connects to its addresses in order until one
    /// accepts; fails with the last address's error if none does.
    pub async fn connect_host(
        rt: &Runtime,
        host: &str,
        port: u16,
        options: &SocketOptions,
    ) -> Result<TcpStream> {
        let mut last = Error::from_hresult(
            windows::Win32::Foundation::WIN32_ERROR(windows::Win32::Networking::WinSock::WSAHOST_NOT_FOUND.0 as u32)
                .to_hresult(),
        );
        for addr in resolve(rt, host, port).await? {
            match TcpStream::connect(rt, addr, options).await {
                Ok(stream) => return Ok(stream),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tp::{block_on, timeout};
    use std::time::Duration;

    #[test]
    fn localhost_resolves_to_loopback() {
        let rt = Runtime::new();
        let addrs = block_on(resolve(&rt, "localhost", 8080)).unwrap();
        assert!(!addrs.is_empty(), "no addresses for localhost");
        assert!(addrs.iter().all(|a| a.ip().is_loopback() && a.port() == 8080), "{addrs:?}");
    }

    #[test]
    fn numeric_addresses_resolve_to_themselves() {
        let rt = Runtime::new();
        let addrs = block_on(resolve(&rt, "192.0.2.7", 53)).unwrap();
        assert_eq!(addrs, vec!["192.0.2.7:53".parse::<SocketAddr>().unwrap()]);
    }

    #[test]
    fn an_unknown_name_fails() {
        let rt = Runtime::new();
        let err = block_on(resolve(&rt, "no-such-host.invalid", 80)).unwrap_err();
        eprintln!("unknown name -> {err:?}");
    }

    /// A lookup dropped mid-flight is cancelled and returns promptly.
    #[test]
    fn a_dropped_lookup_is_cancelled() {
        let rt = Runtime::new();
        let started = std::time::Instant::now();
        let _ = block_on(timeout(&rt, Duration::from_millis(1), resolve(&rt, "example.invalid", 80)));
        assert!(started.elapsed() < Duration::from_secs(10), "the cancel waited too long");
    }

    #[test]
    fn connect_host_reaches_a_local_listener() {
        let rt = Runtime::new();
        let listener = super::super::TcpListener::bind(
            &rt,
            "127.0.0.1:0".parse().unwrap(),
            &SocketOptions::new(),
        )
        .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || block_on(listener.accept()).map(|_| ()));
        let stream = block_on(TcpStream::connect_host(&rt, "localhost", port, &SocketOptions::new()));
        // `localhost` may list ::1 first; a v4-only listener refuses it and
        // the next address connects.
        stream.unwrap();
        server.join().unwrap().unwrap();
    }
}
