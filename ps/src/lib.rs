//! The proxy/stub DLL for `Pbk20191.WinAsync.Comio`.
//!
//! All of the marshalling is MIDL's, compiled from `generated/` by the build
//! script. What is here are the two exports COM looks up, forwarding to the
//! `Prx`-prefixed entry points `dlldata.c` defines -- written in Rust only so
//! that a cdylib has something to link the C into.
//!
//! The DLL holds no Rust state and does no I/O; it does not use the
//! `winasync` library at all.

use core::ffi::c_void;

unsafe extern "system" {
    fn PrxDllGetClassObject(clsid: *const c_void, iid: *const c_void, object: *mut *mut c_void) -> i32;
    fn PrxDllCanUnloadNow() -> i32;
}

/// Hands COM the proxy/stub factory.
///
/// # Safety
///
/// Called by the COM runtime with a valid CLSID, IID and out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const c_void,
    iid: *const c_void,
    object: *mut *mut c_void,
) -> i32 {
    unsafe { PrxDllGetClassObject(clsid, iid, object) }
}

/// The RPC runtime's own answer: whether any proxy or stub it built from this
/// DLL is still alive.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    unsafe { PrxDllCanUnloadNow() }
}
