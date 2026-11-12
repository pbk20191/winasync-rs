//! One COM class in a static library, to measure whether a caller's vtable
//! call into it survives cross-language LTO as an indirect call, or is
//! devirtualized and inlined. The method is deliberately tiny -- an add
//! with a per-object salt -- so that a surviving `call` is the whole cost
//! and inlining is visible in the disassembly.
//!
//! `probe_create_adder` is the only way a caller gets an object, and it is
//! `extern "C"` rather than an activation factory: the question is what the
//! optimizer can do once it holds the interface pointer, not how it got it.
//! `probe_adder_add` is the same method as a plain exported symbol, the
//! upper bound a direct call can reach.

#![allow(non_snake_case)]

use core::ffi::c_void;
use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, Interface, implement, interface};

#[interface("5f4d0b6e-1c7a-4a3e-9d2c-7b1e3f8a9c01")]
pub unsafe trait IAdder: IUnknown {
    fn Add(&self, a: i32, b: i32, out: *mut i32) -> HRESULT;
}

#[implement(IAdder)]
struct Adder {
    salt: i32,
}

impl IAdder_Impl for Adder_Impl {
    unsafe fn Add(&self, a: i32, b: i32, out: *mut i32) -> HRESULT {
        unsafe { *out = a.wrapping_add(b).wrapping_add(self.salt) };
        HRESULT(0)
    }
}

/// A new `Adder` as an `IAdder` interface pointer, refcount 1.
#[unsafe(no_mangle)]
pub extern "system" fn probe_create_adder(salt: i32) -> *mut c_void {
    let obj: IAdder = Adder { salt }.into();
    obj.into_raw()
}

/// `IAdder::Add` as a symbol: `this` is the same interface pointer the
/// vtable call takes (interface 1 of the object, so the object starts one
/// pointer before it, as the generated vtable thunk computes).
///
/// # Safety
/// `this` must be an `IAdder` from [`probe_create_adder`].
#[unsafe(no_mangle)]
pub unsafe extern "system" fn probe_adder_add(this: *mut c_void, a: i32, b: i32, out: *mut i32) -> HRESULT {
    let this = unsafe { &*((this as *const *const ()).offset(-1) as *const Adder_Impl) };
    unsafe { this.Add(a, b, out) }
}
