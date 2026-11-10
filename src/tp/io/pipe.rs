//! Async named-pipe IO over TP_IO with a raw `(Result, Vec<u8>)` API (the
//! `tp`-native counterpart to comio's WinRT-shaped named pipe).
//!
//! Instance creation follows `common::pipe`: the first instance claims the
//! name, one listening instance is always kept, remote clients are refused.
//! The connector opens at identification level, so a server can tell who we
//! are and cannot act as us.

use super::{submit, IoRes, Issue};
use crate::common::buffer::{IoBuf, IoBufMut};
use crate::common::pipe::{PipeServer, ReplenishOnExit};
use crate::common::security::{self, SecurityDescriptor};
use crate::tp::Runtime;
use std::sync::Arc;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{ConnectNamedPipe, WaitNamedPipeW};
use windows_core::HSTRING;

/// A connected named-pipe endpoint (server- or client-side); duplex, sequential.
pub struct NamedPipeStream {
    res: Arc<IoRes>,
    /// Which end this is, which decides how the peer is asked for.
    server_side: bool,
}

impl NamedPipeStream {
    /// The process id at the other end: the client, on a stream from
    /// `accept`; the server, on one from `connect`.
    pub fn peer_process_id(&self) -> windows_core::Result<u32> {
        security::pipe_peer_pid(self.res.handle, self.server_side)
    }

    /// The account at the other end, as a string SID.
    ///
    /// On an accepted stream this is read from the client's own token, so it
    /// is the account the client connected as. On a connected stream it is
    /// the server process's account, which needs the right to open that
    /// process -- usually only true when both run as the same user, which is
    /// exactly the case worth checking: that the server we reached is ours.
    pub fn peer_user_sid(&self) -> windows_core::Result<String> {
        if self.server_side {
            security::pipe_client_sid(self.res.handle)
        } else {
            security::process_user_sid(self.peer_process_id()?)
        }
    }

    /// Reads into `buf`; resolves to the byte count and hands the buffer back.
    pub fn read<B: IoBufMut>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;
        
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice =
                    core::slice::from_raw_parts_mut(buf.stable_mut_ptr(), IoBuf::len(buf));
                match ReadFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }

    /// Writes `buf`; resolves to the byte count and hands the buffer back.
    pub fn write<B: IoBuf>(
        &self,
        buf: B,
    ) -> impl Future<Output = (windows_core::Result<usize>, B)> + Send + use<B> {
        let handle = self.res.handle;
        
        unsafe {
            submit(&self.res, buf, move |ovl, buf| {
                let slice = core::slice::from_raw_parts(buf.stable_ptr(), IoBuf::len(buf));
                match WriteFile(handle, Some(slice), None, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        }
    }
}

/// Server side: each `accept` waits for a client on a listening instance.
pub struct NamedPipeListener {
    rt: Runtime,
    server: PipeServer,
}

impl NamedPipeListener {
    /// Binds a listener to `name` (e.g. `\\.\pipe\foo`), with the default
    /// security: the creator, administrators and SYSTEM. No instance is
    /// created until the first `accept`, which is also where a name already
    /// held by another server is reported.
    pub fn bind(rt: &Runtime, name: &str) -> NamedPipeListener {
        NamedPipeListener {
            rt: rt.clone(),
            server: PipeServer::new(name, None),
        }
    }

    /// [`bind`](Self::bind), with `security` on every instance. It must grant
    /// this process's own account `FILE_CREATE_PIPE_INSTANCE` (`GA` does):
    /// every instance after the first is checked against it.
    pub fn bind_with_security(
        rt: &Runtime,
        name: &str,
        security: &SecurityDescriptor,
    ) -> NamedPipeListener {
        NamedPipeListener {
            rt: rt.clone(),
            server: PipeServer::new(name, Some(security.clone())),
        }
    }

    /// Waits for a client on a listening instance.
    pub async fn accept(&self) -> windows_core::Result<NamedPipeStream> {
        let handle = self.server.instance()?;
        let res = match IoRes::register(&self.rt.inner, handle, false) {
            Ok(res) => res,
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err(e);
            }
        };
        let _replenish = ReplenishOnExit(&self.server);
        let h = handle;
        let fut = unsafe {
            submit(&res, (), move |ovl, _| {
                // ERROR_PIPE_CONNECTED (a client connected between
                // CreateNamedPipeW and here) reports success but queues NO
                // completion, so classify it as Failed — submit then balances
                // the StartThreadpoolIo and delivers it synchronously; accept
                // treats it as "connected" below.
                match ConnectNamedPipe(h, Some(ovl)) {
                    Ok(()) => Issue::SyncSuccess,
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => Issue::Pending,
                    Err(e) => Issue::Failed(e.code()),
                }
            })
        };
        let (result, ()) = fut.await;
        match result {
            Ok(_) => {}
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
            Err(e) => return Err(e),
        }
        Ok(NamedPipeStream {
            res,
            server_side: true,
        })
    }
}

/// Client side.
pub struct NamedPipeConnector;

impl NamedPipeConnector {
    /// Opens the client end of an existing named pipe, waiting once via
    /// `WaitNamedPipeW` if every instance is momentarily busy.
    pub fn connect(rt: &Runtime, name: &str) -> windows_core::Result<NamedPipeStream> {
        let wname = HSTRING::from(name);
        let handle = match Self::open(&wname) {
            Ok(h) => h,
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                unsafe { WaitNamedPipeW(&wname, 5000) }.ok()?;
                Self::open(&wname)?
            }
            Err(e) => return Err(e),
        };
        match IoRes::register(&rt.inner, handle, false) {
            Ok(res) => Ok(NamedPipeStream {
                res,
                server_side: false,
            }),
            Err(e) => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    /// Identification level, not the default impersonation: the server may
    /// learn who we are, but cannot act as us. Without
    /// `SECURITY_SQOS_PRESENT` a client grants full impersonation to whatever
    /// answers on the name.
    fn open(wname: &HSTRING) -> windows_core::Result<HANDLE> {
        unsafe {
            CreateFileW(
                wname,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{block_on, Runtime};
    use super::*;

    #[test]
    fn named_pipe_echo() {
        let rt = Runtime::new();
        let name = format!(r"\\.\pipe\tp_pipe_echo_{}", std::process::id());
        let rt2 = rt.clone();
        let sname = name.clone();
        let server = std::thread::spawn(move || {
            let listener = NamedPipeListener::bind(&rt2, &sname);
            let s = block_on(listener.accept()).unwrap();
            let (n, b) = block_on(s.read(vec![0u8; 16]));
            let n = n.unwrap();
            block_on(s.write(b[..n].to_vec())).0.unwrap();
            n
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        block_on(client.write(b"hello".to_vec())).0.unwrap();
        let (n, b) = block_on(client.read(vec![0u8; 16]));
        assert_eq!(&b[..n.unwrap()], b"hello");
        assert_eq!(server.join().unwrap(), 5);
    }

    fn unique(tag: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\tp_pipe_{tag}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Accepts one client of `name` on a thread, returning the server end.
    fn accept_one(rt: &Runtime, listener: NamedPipeListener) -> std::thread::JoinHandle<windows_core::Result<NamedPipeStream>> {
        let _ = rt;
        std::thread::spawn(move || block_on(listener.accept()))
    }

    /// Both ends name each other, and the server reads the client's account
    /// from its token -- this process, on both counts.
    #[test]
    fn each_end_identifies_the_other() {
        let rt = Runtime::new();
        let name = unique("peer");
        let server = accept_one(&rt, NamedPipeListener::bind(&rt, &name));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let client = NamedPipeConnector::connect(&rt, &name).unwrap();
        let server = server.join().unwrap().unwrap();

        let me = std::process::id();
        let my_sid = security::process_user_sid(me).unwrap();
        assert_eq!(server.peer_process_id().unwrap(), me);
        assert_eq!(server.peer_user_sid().unwrap(), my_sid);
        assert_eq!(client.peer_process_id().unwrap(), me);
        assert_eq!(client.peer_user_sid().unwrap(), my_sid);
    }

    /// A pipe someone else created first is not joined: the first instance
    /// asks to be first, and is refused.
    #[test]
    fn a_squatted_name_is_refused() {
        use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX};
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("squat");
        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(!squatter.is_invalid());
        let err = block_on(NamedPipeListener::bind(&rt, &name).accept()).err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        unsafe { CloseHandle(squatter).unwrap() };
    }

    /// Once a client has connected and gone, the name is still ours: the
    /// next instance was created before the connected one was handed on.
    #[test]
    fn the_name_is_held_between_accepts() {
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
        };
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("held");
        let listener = Arc::new(NamedPipeListener::bind(&rt, &name));
        let l2 = listener.clone();
        let server = std::thread::spawn(move || block_on(l2.accept()));
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(NamedPipeConnector::connect(&rt, &name).unwrap());
        drop(server.join().unwrap().unwrap());

        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(squatter.is_invalid(), "the name was free between accepts");
        drop(listener);
    }

    /// An accept dropped before anyone connects -- here by a timeout -- still
    /// leaves the name held: its instance is replaced on the way out.
    #[test]
    fn a_dropped_accept_keeps_the_name() {
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX,
        };
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };
        let rt = Runtime::new();
        let name = unique("dropped");
        let listener = NamedPipeListener::bind(&rt, &name);
        let timed_out = block_on(crate::tp::timeout(
            &rt,
            std::time::Duration::from_millis(100),
            listener.accept(),
        ))
        .is_err();
        assert!(timed_out);
        let squatter = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(&name),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                None,
            )
        };
        assert!(squatter.is_invalid(), "the name was free after a dropped accept");
        // And the reserved instance still serves the next accept.
        let server = accept_one(&rt, listener);
        let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
        server.join().unwrap().unwrap();
    }

    /// A descriptor that does not grant this account keeps it out.
    #[test]
    fn a_descriptor_keeps_out_who_it_does_not_grant() {
        let rt = Runtime::new();
        let name = unique("deny");
        let system_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;SY)").unwrap();
        let listener = NamedPipeListener::bind_with_security(&rt, &name, &system_only);
        // Nobody is let in, so the accept only ends by timing out -- which
        // drops it and cancels its operation, rather than leaving one live
        // for the leak checks elsewhere in this process to trip over.
        let rt2 = rt.clone();
        let server = std::thread::spawn(move || {
            block_on(crate::tp::timeout(
                &rt2,
                std::time::Duration::from_millis(500),
                listener.accept(),
            ))
            .is_err()
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let err = NamedPipeConnector::connect(&rt, &name).err().unwrap();
        assert_eq!(err.code(), windows::Win32::Foundation::ERROR_ACCESS_DENIED.to_hresult());
        assert!(server.join().unwrap(), "the accept completed for a client it should refuse");
    }

    /// And one that does grant it lets it in.
    #[test]
    fn a_descriptor_admits_who_it_grants() {
        let rt = Runtime::new();
        let name = unique("allow");
        let owner_only = SecurityDescriptor::from_sddl("D:P(A;;GA;;;OW)").unwrap();
        let server = accept_one(&rt, NamedPipeListener::bind_with_security(&rt, &name, &owner_only));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _client = NamedPipeConnector::connect(&rt, &name).unwrap();
        server.join().unwrap().unwrap();
    }
}
