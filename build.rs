

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    if target_os.as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=app.manifest");
        println!("cargo:rerun-if-changed=manifest.rc");
        embed_resource::compile("manifest.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
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