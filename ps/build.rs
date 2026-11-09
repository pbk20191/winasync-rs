//! Compiles the checked-in MIDL output for the target architecture.

use std::path::Path;

/// The proxy/stub's own CLSID: what `ProxyStubClsid32` names for every
/// interface here, and what `DllGetClassObject` answers to.
///
/// **Frozen.** It is written into every installation's registry, so changing
/// it orphans them. Mirrored by `winasync::comio::activation::CLSID_PROXY_STUB`;
/// `tests/oop.rs` proves the two agree by asking this DLL for exactly that
/// class.
///
/// Spelled in the initializer form `PROXY_CLSID_IS` takes. Without it MIDL's
/// `dlldata.c` would use the first interface's IID as the CLSID, which makes
/// the identity of the whole DLL depend on which interface sorts first.
const PROXY_CLSID_IS: &str =
    "{0xa59f0ba5,0x0111,0x463b,{0xb1,0xdb,0x10,0x99,0x43,0xa6,0x3f,0x74}}";

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest = Path::new(&manifest);
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let dir = manifest.join("generated").join(&arch);
    if !dir.is_dir() {
        panic!(
            "no proxy/stub generated for `{arch}`; add it to ARCHES in \
             tools/winmdgen/src/proxy.rs and run `cargo run -p winmdgen`"
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=exports.def");
    println!("cargo:rerun-if-changed={}", dir.display());

    cc::Build::new()
        .files(["comio_p.c", "comio_i.c", "dlldata.c"].map(|f| dir.join(f)))
        .include(&dir)
        // `Prx` prefixes the entry points `dlldata.c` defines, so the Rust
        // exports in src/lib.rs can be the real ones and forward to them.
        .define("ENTRY_PREFIX", "Prx")
        .define("PROXY_CLSID_IS", PROXY_CLSID_IS)
        // The generated code targets NT62 (`midl /target`), and the WinRT
        // headers it includes are gated on it.
        .define("_WIN32_WINNT", "0x0A00")
        .define("WIN32", None)
        // MIDL output is not ours to warn about.
        .warnings(false)
        .compile("winasync_proxy");

    // `rpcrt4` is the NDR engine, but on 64-bit Windows it does not export
    // everything MIDL's output names. The stubless-client and forwarding
    // thunks (`ObjectStublessClientN`, `NdrProxyForwardingFunctionN`) live in
    // combase and are reached through `ole32`; the `HSTRING_User*` marshalling
    // routines through `runtimeobject`.
    println!("cargo:rustc-link-lib=rpcrt4");
    println!("cargo:rustc-link-lib=ole32");
    println!("cargo:rustc-link-lib=runtimeobject");

    let def = manifest.join("exports.def");
    println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
}
