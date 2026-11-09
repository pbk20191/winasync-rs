//! Compiles `metadata/rustwinui3loop.rdl` into the `.winmd` this component
//! publishes, then generates the Rust bindings the crate implements against.
//!
//! Run from the repository root:
//!
//! ```text
//! cargo run -p winmdgen
//! ```
//!
//! Both outputs are checked in. That is deliberate: the `.winmd` is a
//! *shipped artifact* — a consumer needs it to generate a Python or C#
//! projection — and the bindings are what the library compiles against, so a
//! reviewer should see both change in the same commit as the RDL that caused
//! it. Nothing in an ordinary `cargo build` runs this.

use std::path::{Path, PathBuf};

/// `metadata/rustwinui3loop.rdl` → `metadata/rustwinui3loop.winmd`.
const RDL: &str = "metadata/rustwinui3loop.rdl";
const WINMD: &str = "metadata/rustwinui3loop.winmd";
/// Generated; do not edit. Lives beside the module that implements it.
const BINDINGS: &str = "src/comio/bindings.rs";

fn main() {
    let root = repo_root();

    let rdl = root.join(RDL);
    let winmd = root.join(WINMD);
    let bindings = root.join(BINDINGS);

    windows_rdl::reader()
        // `Windows.Foundation`, `Windows.Storage.Streams` and the attribute
        // vocabulary our RDL names. Without this the compiler cannot resolve
        // `IAsyncAction` or `Activatable` and says only "type not found".
        .reference_default()
        .input(&rdl)
        .output(&winmd)
        .write()
        .unwrap_or_else(|e| panic!("{RDL}: {e}"));

    println!("wrote {}", winmd.display());

    let warnings = windows_bindgen::bindgen([
        "--in",
        winmd.to_str().expect("winmd path is not UTF-8"),
        "default",
        "--out",
        bindings.to_str().expect("bindings path is not UTF-8"),
        "--filter",
        "Rustwinui3loop",
        // Emit the `*_Impl` traits and `*_Vtbl::new` constructors, i.e. the
        // *implementer's* side. Without it we would get a consumer-only
        // projection and have nothing for `#[implement]` to attach to.
        "--implement",
        // Everything under `Windows.` comes from the `windows` crate we
        // already depend on, rather than being regenerated into our bindings.
        // Leaving this off makes bindgen silently skip every member that
        // mentions `TypedEventHandler` or `IAsyncOperation` — which is to say,
        // the whole point of the exercise.
        "--reference",
        "windows,skip-root,Windows",
    ]);

    let warnings = format!("{warnings:?}");
    // `Warnings` has no accessor; its `Debug` is the only way in. An empty
    // list is the expected state, and a skipped member is never benign here —
    // it means a signature silently vanished from the ABI.
    assert!(
        warnings.contains("Warnings([])"),
        "bindgen reported warnings, which here means dropped members:\n{warnings}"
    );

    println!("wrote {}", bindings.display());
}

/// The repository root, derived from this tool's own manifest rather than the
/// current directory, so `cargo run -p winmdgen` works from anywhere.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("tools/winmdgen is two levels below the repository root")
        .to_path_buf()
}
