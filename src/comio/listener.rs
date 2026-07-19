//! WinRT-native StreamSocketListener-shaped listener over comio transports.
#![allow(dead_code)]

use core::ffi::c_void;
use windows_core::{
    HRESULT, HSTRING, IInspectable, IUnknown, IUnknown_Vtbl, Interface, implement, interface,
};

/// RO_E_CLOSED — returned by members of a closed listener/stream.
pub(crate) const RO_E_CLOSED: HRESULT = HRESULT(0x80000013u32 as i32);

/// Custom fixed-IID interface exposing the stream/endpoint pair delivered by
/// a `ConnectionReceived` event. GUID frozen here — do not change or reuse.
#[interface("9d359f23-57af-4770-b11a-e223be76adc3")]
pub(crate) unsafe trait IConnectionReceivedArgs: IUnknown {
    fn Stream(&self, result: *mut *mut c_void) -> HRESULT;
    fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT;
}

#[implement(IConnectionReceivedArgs)]
struct ConnectionReceivedArgs {
    stream: IInspectable,
    endpoint: HSTRING,
}

impl IConnectionReceivedArgs_Impl for ConnectionReceivedArgs_Impl {
    unsafe fn Stream(&self, result: *mut *mut c_void) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32); // E_POINTER
        }
        unsafe { *result = self.stream.clone().into_raw() };
        HRESULT(0)
    }
    unsafe fn RemoteEndpoint(&self, result: *mut core::mem::ManuallyDrop<HSTRING>) -> HRESULT {
        if result.is_null() {
            return HRESULT(-2147467261i32); // E_POINTER
        }
        unsafe { *result = core::mem::ManuallyDrop::new(self.endpoint.clone()) };
        HRESULT(0)
    }
}

/// Constructs a new `IConnectionReceivedArgs` wrapping `stream`/`endpoint`.
pub(crate) fn new_args(stream: IInspectable, endpoint: HSTRING) -> IConnectionReceivedArgs {
    ConnectionReceivedArgs { stream, endpoint }.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comio::buffer::new_buffer;
    use windows_core::Interface;

    #[test]
    fn args_exposes_stream_and_endpoint() {
        // Any IInspectable stands in for the accepted stream here.
        let stream_stub: windows_core::IInspectable = new_buffer(4).unwrap().into();
        let args = new_args(stream_stub.clone(), windows_core::HSTRING::from("pipe://x"));
        // Stream() returns the same object identity.
        let got = unsafe {
            let mut raw = core::ptr::null_mut();
            assert!(args.Stream(&mut raw).is_ok());
            windows_core::IInspectable::from_raw(raw)
        };
        assert_eq!(got, stream_stub);
        let ep = unsafe {
            let mut s = core::mem::ManuallyDrop::new(windows_core::HSTRING::new());
            assert!(args.RemoteEndpoint(&mut s).is_ok());
            core::mem::ManuallyDrop::into_inner(s)
        };
        assert_eq!(ep.to_string(), "pipe://x");
    }
}
