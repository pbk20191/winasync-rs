//! The COM / WinRT in-process server for `winasync`'s transports.
//!
//! Nothing but the entry points a loader looks up by name. Every body lives
//! in the library, which keeps the logic testable from an ordinary rlib; what
//! this crate adds is the one thing the library must not have, which is
//! exported symbols.
//!
//! # Why a separate package
//!
//! The library used to be `crate-type = ["lib", "cdylib"]`, which made one
//! rustc invocation emit both artifacts. Nothing could tell them apart: there
//! is no `cfg` for crate type, `required-features` does not apply to `[lib]`,
//! and a build script cannot see it. So "only the DLL exports these" and
//! "only the DLL keeps the unload counts" were conventions, and a plain
//! `cargo build` of the library produced a DLL with COM exports whether
//! anyone wanted one or not.
//!
//! Here the only crate type is `cdylib`. Compiling this package is building
//! the server; the library compiled for any other consumer has no exports, no
//! `DllMain`, and -- because the `dylib` feature is enabled by this package's
//! dependency declaration and nowhere else -- no counters.
//!
//! One gap remains, and it is cargo's rather than ours: features unify within
//! a single build. `cargo build --workspace` compiles the library once, with
//! `dylib` on, for every member that depends on it. Build the server with
//! `-p winasync-dll` and a consumer separately and each gets what it asked
//! for.
//!
//! # `DllMain` is in the export table
//!
//! It has no business there: the CRT reaches it by link-time name, not by
//! `GetProcAddress`. But rustc exports every `no_mangle` symbol from a cdylib
//! and `pub` is not the lever. The alternative is a `.CRT$XLB` TLS callback,
//! which is not exported and gets the same notifications, but fails *silently*
//! if the linker drops `_tls_used`. A visible export beats a diagnostic that
//! might not be running.

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use library::comio::{activation, unload};
use windows_core::{GUID, HRESULT, HSTRING};

/// Hands COM the class object for `clsid`.
///
/// # Safety
///
/// Called by the COM runtime with a valid CLSID, IID and out-pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    unsafe { activation::dll_get_class_object(clsid, iid, object) }
}

/// Hands the Windows Runtime the activation factory for `name`.
///
/// # Safety
///
/// Called by the Windows Runtime with a borrowed `HSTRING` and a valid
/// out-pointer. The string stays the caller's, hence `ManuallyDrop`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetActivationFactory(
    name: ManuallyDrop<HSTRING>,
    factory: *mut *mut c_void,
) -> HRESULT {
    unsafe { activation::dll_get_activation_factory(name, factory) }
}

/// `S_OK` only when nothing this module owns can still execute. Nothing to
/// give back at that point: each socket released its own Winsock reference
/// as it closed.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    activation::dll_can_unload_now()
}

/// Load and unload notifications. Records this module's handle on attach
/// -- the `RaceDll` the thread pool pins -- and observes on detach; see
/// `unload::dll_main`.
///
/// # Safety
///
/// Called by the loader. Exported, so others can call it too; a forged
/// attach is ignored, as the comment below explains.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    instance: *mut c_void,
    reason: u32,
    reserved: *mut c_void,
) -> i32 {
    // SAFETY: this is the DLL's `DllMain`. Being exported, it can also be
    // called by anyone; `dll_main` keeps only the first attach, which is the
    // loader's, so a forged call cannot replace the handle.
    unsafe { unload::dll_main(instance, reason, reserved) }
}
