//! The WinRT component as a static library.
//!
//! The DLL (`dll/`) is reached the way the Windows Runtime reaches any
//! component: `RoGetActivationFactory` finds `DllGetActivationFactory` by
//! its reserved name. A static library has no loader step, so none of that
//! applies, and the reserved names are deliberately absent here: nothing in
//! this crate is named `Dll*`, and nothing calls `Ro*` for its own classes.
//! What a caller gets instead is
//!
//! - two plain entry points, [`winasync_get_activation_factory`] and
//!   [`winasync_get_class_object`], over the same class table the DLL's
//!   exports use (`comio::activation`), and
//! - with the library's `static-exports` feature, which only this crate
//!   turns on, one exported function per class and interface returning the
//!   class's vtable (`Pbk20191_WinAsync_Comio_<Class>_<Interface>_vtable`).
//!
//! The second is the point of the crate. A call through an object's vtable
//! is a load from the object, and the optimizer cannot see what it will
//! find there; a call through a table it got from a function it inlined is
//! a load from a constant, which folds, so under full LTO the method
//! inlines into the caller (`tools/lto-probe`, and `probe/` here). The
//! table is the one the object carries, so `this` is the same interface
//! pointer either way. The caller is bound to the class by name, which is
//! the trade: this is for a program that links this library and nothing
//! else as the implementation of those classes.
//!
//! The declarations for C and C++ are in `include/winasync_static.h`.

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use library::comio::activation;
use windows_core::{GUID, HRESULT, HSTRING};

/// The activation factory for a runtime class, by name; the static
/// library's `DllGetActivationFactory`, with the same contract and the
/// same class table.
///
/// # Safety
/// `name` is a valid `HSTRING` (the caller's; not released here) and
/// `factory` a valid out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn winasync_get_activation_factory(
    name: ManuallyDrop<HSTRING>,
    factory: *mut *mut c_void,
) -> HRESULT {
    unsafe { activation::dll_get_activation_factory(name, factory) }
}

/// The class object for a CLSID; the static library's `DllGetClassObject`.
///
/// # Safety
/// `clsid` and `iid` point to GUIDs and `object` is a valid out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn winasync_get_class_object(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    unsafe { activation::dll_get_class_object(clsid, iid, object) }
}
