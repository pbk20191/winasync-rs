//! The proxy/stub: `.winmd` → IDL → C, through the SDK's own tools.
//!
//! An interface only crosses a process boundary if something can marshal its
//! calls, and for a WinRT interface that something is a MIDL-generated
//! proxy/stub. There is no metadata-driven marshaler for third-party WinRT
//! interfaces; the system's own interfaces have proxies registered by
//! Windows, ours need one we ship (`ps/`, `winasync_ps.dll`).
//!
//! The pipeline is the one the SDK documents for a component authored as a
//! `.winmd`: `winmdidl` recovers an IDL from the metadata -- including the
//! `declare { }` block for the parameterized instances the surface uses, so
//! `IAsyncOperation<ITransportStream>` gets a proxy like everything else --
//! and `midl /winrt` compiles that to C.
//!
//! Like the `.winmd` and the bindings, the output is checked in. An ordinary
//! build compiles it with the C compiler the Rust MSVC toolchain already
//! needs, and never runs `midl`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One target architecture. The C differs per architecture -- x64 and ARM64
/// stubs carry NDR64 format strings -- so each gets its own directory and
/// `ps/build.rs` picks by `CARGO_CFG_TARGET_ARCH`.
pub struct Arch {
    /// `midl /env`.
    env: &'static str,
    /// Subdirectory of `ps/generated`, spelled as `CARGO_CFG_TARGET_ARCH`.
    pub dir: &'static str,
}

pub const ARCHES: &[Arch] = &[
    Arch { env: "x64", dir: "x86_64" },
    Arch { env: "arm64", dir: "aarch64" },
];

/// The IDL file name `winmdidl` derives from the `.winmd`. It is also what the
/// generated sources name as their input, once [`scrub`] has removed the
/// temporary directory midl actually read it from.
const IDL: &str = "Pbk20191.WinAsync.Comio.idl";

pub fn generate(winmd: &Path, out: &Path, arch: &Arch) {
    let sdk = Sdk::find();
    let work = std::env::temp_dir().join(format!("winmdgen-{}-{}", std::process::id(), arch.dir));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("create work dir");
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).expect("create proxy output dir");

    run(
        sdk.command("winmdidl.exe")
            .arg("/nologo")
            .arg(format!("/outdir:{}", work.display()))
            .arg(format!("/metadata_dir:{}", sdk.union_metadata().display()))
            .arg(winmd),
        "winmdidl",
    );

    run(
        sdk.command("midl.exe")
            .args(["/nologo", "/winrt", "/W1", "/char", "signed", "/ns_prefix"])
            // Windows 8 is the floor for WinRT marshalling in any case.
            .args(["/target", "NT62", "/env", arch.env])
            .arg("/metadata_dir")
            .arg(sdk.union_metadata())
            .arg("/I")
            .arg(sdk.include("winrt"))
            .arg("/I")
            .arg(sdk.include("shared"))
            .arg("/I")
            .arg(sdk.include("um"))
            .arg("/out")
            .arg(out)
            .args(["/h", "comio.h", "/iid", "comio_i.c", "/proxy", "comio_p.c"])
            .args(["/dlldata", "dlldata.c"])
            // midl also re-emits a `.winmd` from the IDL. It is not ours to
            // ship -- the one `windows-rdl` wrote is -- so it goes to scratch.
            .arg("/winmd")
            .arg(work.join("discard.winmd"))
            .arg(work.join(IDL)),
        "midl",
    );

    for entry in std::fs::read_dir(out).expect("read proxy output dir") {
        scrub(&entry.expect("dir entry").path());
    }
    let _ = std::fs::remove_dir_all(&work);
}

/// Makes the output reproducible.
///
/// midl records the path of the IDL it compiled, and it compiles a copy in a
/// randomly named temporary file -- so every run would rewrite every file with
/// a new user-specific path. The timestamp needs no help: midl already pins it
/// to a fixed date.
fn scrub(path: &Path) {
    const MARK: &str = "Compiler settings for ";
    let text = std::fs::read_to_string(path).expect("read generated file");
    let mut changed = false;
    let lines: Vec<String> = text
        .lines()
        .map(|line| match line.find(MARK) {
            Some(at) => {
                changed = true;
                format!("{}{MARK}{IDL}:", &line[..at])
            }
            None => line.to_owned(),
        })
        .collect();
    if changed {
        let mut text = lines.join("\r\n");
        text.push_str("\r\n");
        std::fs::write(path, text).expect("write generated file");
    }
}

fn run(cmd: &mut Command, what: &str) {
    let out = cmd.output().unwrap_or_else(|e| panic!("could not start {what}: {e}"));
    if !out.status.success() {
        panic!(
            "{what} failed ({}):\n{}{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// The Windows SDK the MSVC toolchain would compile against.
struct Sdk {
    /// `...\Windows Kits\10`
    root: PathBuf,
    /// `10.0.26100.0`
    version: String,
    /// The compiler environment: midl runs `cl.exe` to preprocess, and finds
    /// it on this `PATH`.
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

impl Sdk {
    fn find() -> Self {
        let cl = cc::windows_registry::find_tool("x86_64-pc-windows-msvc", "cl.exe")
            .expect("no MSVC toolchain found; midl needs cl.exe to preprocess");
        let env: Vec<_> = cl.env().to_vec();
        let include = env
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("INCLUDE"))
            .map(|(_, v)| v.clone())
            .expect("the MSVC environment has no INCLUDE");
        // One entry is `<root>\Include\<version>\um`; the rest of the SDK is
        // laid out relative to it.
        let um = std::env::split_paths(&include)
            .find(|p| {
                p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("um"))
                    && p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("Windows Kits"))
            })
            .expect("no Windows SDK `um` directory on INCLUDE");
        let version_dir = um.parent().expect("SDK version dir");
        let version = version_dir
            .file_name()
            .expect("SDK version")
            .to_string_lossy()
            .into_owned();
        let root = version_dir
            .parent()
            .and_then(Path::parent)
            .expect("SDK root")
            .to_path_buf();
        Sdk { root, version, env }
    }

    fn command(&self, tool: &str) -> Command {
        let mut cmd = Command::new(self.root.join("bin").join(&self.version).join("x64").join(tool));
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd
    }

    fn union_metadata(&self) -> PathBuf {
        self.root.join("UnionMetadata").join(&self.version)
    }

    fn include(&self, part: &str) -> PathBuf {
        self.root.join("Include").join(&self.version).join(part)
    }
}
