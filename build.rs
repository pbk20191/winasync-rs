

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    if target_os.as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=manifest.rc");
        embed_resource::compile("manifest.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();

        // The cdylib is the COM in-process server. Its four `Dll*` entry
        // points are reached only by the COM loader's `GetProcAddress`, so
        // the .def marks them PRIVATE to keep them out of the import
        // library; without it MSVC warns LNK4104 on every build. This is a
        // `cdylib-link-arg`, so the rlib, the demo bin and the test harness
        // are untouched.
        // Gated on `comio`, which is what defines the four symbols: naming
        // them in a .def for a build that does not have them is an
        // unresolved-external link error, not a missing export.
        if std::env::var_os("CARGO_FEATURE_COMIO").is_some()
            && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        {
            println!("cargo:rerun-if-changed=comserver.def");
            let def = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("comserver.def");
            println!("cargo:rustc-cdylib-link-arg=/DEF:{}", def.display());
        }
    }
    // https://github.com/microsoft/windows-app-rs
//     windows_bindgen::bindgen(
//
//         [
//             // "--no-deps",
//             "--in",
//             "default",
//             "winmd",
//             //
// // "--sys",
//             "--reference", "windows_future,flat,Windows.Foundation.IAsyncAction",
//
//             "--reference", "windows,skip-root,Windows.Foundation.*",
//             // "--reference", "windows,full,Windows.Foundation.TypedEventHandler",
//
//             // "Windows.Foundation",
//             "--out",
//             "src/bindings.rs",
//             // "--package",
//             // "--implement",
//             // "--specific-deps",
//                         // "--flat",
//                             // "--specific-deps",
//             "--filter",
//             // "DispatcherQueue",
//             // "DispatcherQueueController",
//             "Microsoft.UI.Dispatching",
//             // "!Microsoft.UI.Dispatching.IDispatcherQueueTimer",
//             // "!IDispatcherQueueShutdownStartingEventArgs",
//             // "!IDispatcherQueue3"
//
//         ]
//     ).unwrap();
//
//     ::windows_app::bootstrap::deploy::to_output_dir();
//
//     // Temporary workaround for https://github.com/microsoft/WindowsAppSDK/issues/2634
//     ::windows_app::build::embed_manifest(
//         r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
//            <assembly manifestVersion="1.0" xmlns="urn:schemas-microsoft-com:asm.v1" />"#,
//     )
}