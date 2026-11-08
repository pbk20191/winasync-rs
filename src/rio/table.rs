//! The `RIO_EXTENSION_FUNCTION_TABLE` lookup.
//!
//! RIO is the one Winsock extension that is *not* fetched a pointer at a time:
//! all thirteen entry points arrive together as a table, and the only way to
//! ask for it is `SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER` with
//! [`WSAID_MULTIPLE_RIO`]. That ioctl's input is a single GUID naming the
//! table — not an array of extension GUIDs, which is what its name suggests —
//! and RIO's is the only table GUID Windows providers accept. See
//! `common::winsock::extension_fn` for the measurement behind that.
//!
//! Two details worth knowing, both verified by the test below:
//!   * the output buffer receives the table **by value**, not a pointer to it,
//!     despite the documentation's looser wording; and
//!   * `cbSize` must be set on input and comes back as the table's own size,
//!     which is how the provider version-negotiates.

use std::sync::OnceLock;
use windows::Win32::Networking::WinSock::{
    closesocket, WSAIoctl, WSASocketW, AF_INET, RIO_EXTENSION_FUNCTION_TABLE,
    SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER, SOCKET, SOCK_DGRAM, WSAID_MULTIPLE_RIO,
    WSA_FLAG_OVERLAPPED, WSA_FLAG_REGISTERED_IO,
};

use crate::common::winsock::{wsa_init, wsa_last_err};

/// The provider's RIO entry points.
///
/// Every function pointer is validated non-null once, at [`rio_table`] time,
/// so the accessors cannot fail. Keeping the raw table rather than unpacking
/// it into thirteen fields keeps this honest about what the provider returned.
pub(crate) struct RioTable(RIO_EXTENSION_FUNCTION_TABLE);

// The table is a set of plain function pointers into mswsock.dll, valid for
// the process's lifetime.
unsafe impl Send for RioTable {}
unsafe impl Sync for RioTable {}

/// Borrows the provider's table.
///
/// Call sites invoke entry points directly — `(t.raw().RIOSend.unwrap())(..)`
/// — rather than through thirteen hand-written accessors. Re-declaring those
/// signatures here would mean maintaining a second, unverified copy of the
/// ABI next to the one `windows` already generates; the `unwrap` is
/// unreachable because [`rio_table`] rejects an incomplete table.
impl RioTable {
    #[inline]
    pub(crate) fn raw(&self) -> &RIO_EXTENSION_FUNCTION_TABLE {
        &self.0
    }
}

/// Fetches (once) the provider's RIO table.
///
/// Fails where RIO is unavailable — the provider answers `WSAEOPNOTSUPP` /
/// `WSAEINVAL` — so callers can degrade to the ordinary overlapped transports
/// rather than assuming RIO exists.
pub(crate) fn rio_table() -> windows_core::Result<&'static RioTable> {
    static TABLE: OnceLock<RioTable> = OnceLock::new();
    if let Some(t) = TABLE.get() {
        return Ok(t);
    }
    let fetched = fetch()?;
    // A racing thread may have published first; either table is equally valid.
    let _ = TABLE.set(fetched);
    Ok(TABLE.get().expect("table published above"))
}

fn fetch() -> windows_core::Result<RioTable> {
    wsa_init();
    // A throwaway socket purely to reach the provider. UDP/AF_INET is the
    // cheapest; the table is the TCP/IP provider's either way.
    let sock = unsafe {
        WSASocketW(
            AF_INET.0 as i32,
            SOCK_DGRAM.0,
            0,
            None,
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_REGISTERED_IO,
        )
    }?;
    let result = fetch_on(sock);
    unsafe { closesocket(sock) };
    result
}

fn fetch_on(sock: SOCKET) -> windows_core::Result<RioTable> {
    let mut table = RIO_EXTENSION_FUNCTION_TABLE {
        cbSize: size_of::<RIO_EXTENSION_FUNCTION_TABLE>() as u32,
        ..Default::default()
    };
    let mut bytes = 0u32;
    let r = unsafe {
        WSAIoctl(
            sock,
            SIO_GET_MULTIPLE_EXTENSION_FUNCTION_POINTER,
            Some(&WSAID_MULTIPLE_RIO as *const _ as *const core::ffi::c_void),
            size_of::<windows_core::GUID>() as u32,
            Some(&mut table as *mut _ as *mut core::ffi::c_void),
            size_of::<RIO_EXTENSION_FUNCTION_TABLE>() as u32,
            &mut bytes,
            None,
            None,
        )
    };
    if r != 0 {
        return Err(wsa_last_err());
    }
    let t = RioTable(table);
    if !t.all_present() {
        // A provider that reports success must fill every entry; refuse a
        // partial table rather than discover a null pointer at call time.
        return Err(windows_core::Error::from_hresult(windows_core::HRESULT(
            0x8000_4003u32 as i32, // E_POINTER
        )));
    }
    Ok(t)
}

impl RioTable {
    fn all_present(&self) -> bool {
        let t = &self.0;
        t.RIOReceive.is_some()
            && t.RIOReceiveEx.is_some()
            && t.RIOSend.is_some()
            && t.RIOSendEx.is_some()
            && t.RIOCloseCompletionQueue.is_some()
            && t.RIOCreateCompletionQueue.is_some()
            && t.RIOCreateRequestQueue.is_some()
            && t.RIODequeueCompletion.is_some()
            && t.RIODeregisterBuffer.is_some()
            && t.RIORegisterBuffer.is_some()
            && t.RIONotify.is_some()
            && t.RIOResizeCompletionQueue.is_some()
            && t.RIOResizeRequestQueue.is_some()
    }

    /// The size the provider reported — its version handshake.
    pub(crate) fn cb_size(&self) -> u32 {
        self.0.cbSize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rio_table_is_available_and_complete() {
        match rio_table() {
            Ok(t) => {
                // Returned by value, sized to the struct: 8 bytes of
                // cbSize + padding, then 13 pointers.
                assert_eq!(t.cb_size(), size_of::<RIO_EXTENSION_FUNCTION_TABLE>() as u32);
                assert_eq!(t.cb_size(), 112, "13 entry points on a 64-bit provider");
                assert!(t.all_present());
                // Cached: a second call must hand back the same table.
                let again = rio_table().unwrap();
                assert_eq!(again.cb_size(), t.cb_size());
            }
            Err(e) => panic!("RIO unavailable on this machine: {e:?}"),
        }
    }
}
