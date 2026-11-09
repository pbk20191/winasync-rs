fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=exports.def");

    // `/DEF` is an MSVC linker option. On any other toolchain the exports
    // still come from `no_mangle`; they just also land in the import library.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let def = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("exports.def");
        println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
    }
}
