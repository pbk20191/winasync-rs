//! Access control and peer identity for the local transports -- named pipes
//! and AF_UNIX -- shared by `tp` and `comio`.
//!
//! Two questions a local service asks, and where Windows answers them:
//!
//! - **Who may connect.** A [`SecurityDescriptor`], written as SDDL. A named
//!   pipe takes it at `CreateNamedPipeW`; an AF_UNIX socket has none of its
//!   own, so its DACL goes on the socket file, which is what `connect` is
//!   checked against.
//! - **Who did connect.** The peer's process id, which both transports
//!   report, and the account it connected as. A pipe server reads the
//!   account from the client's own token by impersonating it at
//!   identification level; AF_UNIX reports only a process id, so the account
//!   is that process's token, when the server may open it.

use core::ffi::c_void;
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SDDL_REVISION_1,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetTokenInformation,
    OBJECT_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    RevertToSelf, SE_DACL_PRESENT, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SetFileSecurityW,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, OpenProcess, OpenProcessToken, OpenThreadToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_core::{Error, HSTRING, PWSTR, Result};

/// A self-relative security descriptor parsed from SDDL.
///
/// Cheap to clone: the descriptor is immutable once parsed, so clones share
/// one allocation.
#[derive(Clone)]
pub struct SecurityDescriptor(Arc<Inner>);

struct Inner {
    ptr: PSECURITY_DESCRIPTOR,
    sddl: String,
}

// The descriptor is a `LocalAlloc`ed self-relative block that nothing writes
// after `ConvertStringSecurityDescriptorToSecurityDescriptorW` returns it, so
// reading it from any thread is sound.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.ptr.0)));
        }
    }
}

impl std::fmt::Debug for SecurityDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecurityDescriptor").field(&self.0.sddl).finish()
    }
}

impl SecurityDescriptor {
    /// Parses `sddl`, e.g. `D:P(A;;GA;;;OW)(A;;GA;;;SY)` -- the owner and
    /// SYSTEM, nothing inherited. Fails with the parser's own error for
    /// anything that is not SDDL.
    pub fn from_sddl(sddl: &str) -> Result<Self> {
        let mut ptr = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut ptr,
                None,
            )?;
        }
        Ok(Self(Arc::new(Inner {
            ptr,
            sddl: sddl.to_owned(),
        })))
    }

    /// The SDDL this was parsed from, as given.
    pub fn sddl(&self) -> &str {
        &self.0.sddl
    }

    /// `SECURITY_ATTRIBUTES` pointing at this descriptor, for a create call.
    /// Borrows `self`: the result must not outlive it.
    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.ptr.0,
            bInheritHandle: false.into(),
        }
    }

    /// Writes this descriptor's DACL onto the file at `path`.
    ///
    /// Only the DACL: an owner or SACL in the SDDL is not applied, because
    /// changing either needs privileges a service should not need to hold
    /// just to listen. A descriptor with no DACL at all leaves the file's
    /// inherited one alone rather than writing a NULL DACL, which would grant
    /// everyone everything -- `D:NO_ACCESS_CONTROL` still asks for that
    /// explicitly. `D:P` (protected) is honoured, so inherited entries from
    /// the directory do not survive.
    pub(crate) fn apply_dacl_to_file(&self, path: &str) -> Result<()> {
        let mut control = 0u16;
        let mut revision = 0u32;
        unsafe { GetSecurityDescriptorControl(self.0.ptr, &mut control, &mut revision)? };
        if control & SE_DACL_PRESENT.0 == 0 {
            return Ok(());
        }
        let mut info: OBJECT_SECURITY_INFORMATION = DACL_SECURITY_INFORMATION;
        if control & SE_DACL_PROTECTED.0 != 0 {
            info |= PROTECTED_DACL_SECURITY_INFORMATION;
        }
        unsafe { SetFileSecurityW(&HSTRING::from(path), info, self.0.ptr) }.ok()
    }
}

/// The string SID of the user a token belongs to.
fn token_user_sid(token: HANDLE) -> Result<String> {
    // Two calls: the first sizes the buffer and is expected to fail.
    let mut len = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    // `u64` elements so the buffer is aligned for `TOKEN_USER`'s pointer.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            len,
            &mut len,
        )?;
    }
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut text = PWSTR::null();
    unsafe {
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let sid = text
            .to_string()
            .map_err(|_| Error::from_hresult(windows::Win32::Foundation::E_UNEXPECTED));
        let _ = LocalFree(Some(HLOCAL(text.0 as *mut c_void)));
        sid
    }
}

/// Closes a handle when it goes out of scope.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The account process `pid` runs as.
///
/// Needs `PROCESS_QUERY_LIMITED_INFORMATION` on the process and
/// `TOKEN_QUERY` on its token, which a server running as the same user has
/// and a server running as someone else usually does not. A process id can
/// also be reused once its process exits, so this names whoever holds the id
/// *now*; callers that need the account the peer connected as should prefer
/// a transport that reports it (a named pipe does).
pub(crate) fn process_user_sid(pid: u32) -> Result<String> {
    let process = Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)? });
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token)? };
    let token = Owned(token);
    token_user_sid(token.0)
}

/// The account the client of a connected named-pipe server instance
/// connected as, read from its token.
///
/// This impersonates the client on the calling thread for exactly as long as
/// it takes to open that token. A client of ours grants identification level
/// only (`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`), which is enough
/// to read the token and not enough to act with it; an anonymous client
/// grants nothing to read, and this fails.
pub(crate) fn pipe_client_sid(pipe: HANDLE) -> Result<String> {
    unsafe { windows::Win32::System::Pipes::ImpersonateNamedPipeClient(pipe)? };
    let mut token = HANDLE::default();
    // `OpenAsSelf`: the access check for opening the token is made against
    // the process, not the client being impersonated.
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) };
    // Revert before anything else can run on this thread. A pool thread left
    // wearing a client's identity would run the next work item as that
    // client; there is no safe way to carry on from that, so it ends here.
    if unsafe { RevertToSelf() }.is_err() {
        std::process::abort();
    }
    opened?;
    let token = Owned(token);
    token_user_sid(token.0)
}

/// The process id at the other end of a connected named pipe, from either
/// end: a server instance asks for its client, a client handle for the
/// server.
pub(crate) fn pipe_peer_pid(pipe: HANDLE, server_side: bool) -> Result<u32> {
    let mut pid = 0u32;
    unsafe {
        if server_side {
            windows::Win32::System::Pipes::GetNamedPipeClientProcessId(pipe, &mut pid)?;
        } else {
            windows::Win32::System::Pipes::GetNamedPipeServerProcessId(pipe, &mut pid)?;
        }
    }
    Ok(pid)
}

/// The process id at the other end of a connected AF_UNIX socket, from
/// either end (`SIO_AF_UNIX_GETPEERPID`). The only peer credential the
/// Windows AF_UNIX provider reports: there is no `SO_PEERCRED`.
pub(crate) fn unix_peer_pid(sock: windows::Win32::Networking::WinSock::SOCKET) -> Result<u32> {
    use windows::Win32::Networking::WinSock::{SIO_AF_UNIX_GETPEERPID, WSAIoctl};
    let mut pid = 0u32;
    let mut returned = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_AF_UNIX_GETPEERPID,
            None,
            0,
            Some(&mut pid as *mut u32 as *mut c_void),
            size_of::<u32>() as u32,
            &mut returned,
            None,
            None,
        )
    };
    crate::common::winsock::sock_check(r)?;
    Ok(pid)
}

/// Who connected, as the accept path reports it: best effort, so a peer the
/// server cannot identify still connects, and reports `0` / an empty SID.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerIdentity {
    /// The peer's process id, or 0 if the transport could not say.
    pub process_id: u32,
    /// The peer's account as a string SID (`S-1-5-21-...`), or empty.
    pub user_sid: String,
}

impl PeerIdentity {
    /// The client of a connected named-pipe server instance.
    pub(crate) fn of_pipe_client(pipe: HANDLE) -> Self {
        Self {
            process_id: pipe_peer_pid(pipe, true).unwrap_or(0),
            user_sid: pipe_client_sid(pipe).unwrap_or_default(),
        }
    }

    /// The peer of a connected AF_UNIX socket.
    pub(crate) fn of_unix_peer(sock: windows::Win32::Networking::WinSock::SOCKET) -> Self {
        let process_id = unix_peer_pid(sock).unwrap_or(0);
        let user_sid = if process_id == 0 {
            String::new()
        } else {
            process_user_sid(process_id).unwrap_or_default()
        };
        Self { process_id, user_sid }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sddl_round_trips_and_rejects_garbage() {
        let sd = SecurityDescriptor::from_sddl("D:P(A;;GA;;;OW)(A;;GA;;;SY)").unwrap();
        assert_eq!(sd.sddl(), "D:P(A;;GA;;;OW)(A;;GA;;;SY)");
        assert!(SecurityDescriptor::from_sddl("not sddl").is_err());
    }

    #[test]
    fn own_process_has_a_user_sid() {
        let sid = process_user_sid(std::process::id()).unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
    }
}
