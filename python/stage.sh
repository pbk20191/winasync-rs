#!/usr/bin/env bash
# Builds the component and stages a Python projection of it beside this script.
#
# Everything here is generated, so nothing it produces is checked in. Run it
# once after `cargo run -p winmdgen` changes the metadata.
#
#   tools:  dotnet tool restore            (PyWinRT, pinned in dotnet-tools.json)
#   python: pip install -r requirements.txt
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
PY_EXE="${PY_EXE:-python}"

NAMESPACE="Pbk20191.WinAsync.Comio"
# Relative, and passed from `$HERE`. `$REPO` is a Git Bash path (`/e/...`),
# which pywinrt is a .NET tool and cannot resolve; MSYS does not rewrite it
# either, because `<package>;<path>` is one argument and the semicolon reads
# as a path-list separator. A relative path sidesteps both.
WINMD="../metadata/$NAMESPACE.winmd"

# The server DLL is its own package, `dll/`; the integration test is what
# writes the reg-free manifest out of `activation::manifest()` beside it.
(cd "$REPO" && cargo build -p winasync-dll && cargo test --features comio --test activation)

# winmd -> Python projection. `--component-dlls` says the component ships
# inside the package, which is what the copy below arranges.
rm -rf "$HERE/out"
(cd "$HERE" && dotnet pywinrt \
    --input "winasync;$WINMD" \
    --reference sdk \
    --output out \
    --include "${NAMESPACE%%.*}" \
    --component-dlls)

# Where the package landed. Found rather than spelled out: the directory
# layout is PyWinRT's own, it encodes the namespace, and it moved under us
# once already when the namespace was renamed. The projection table is the
# one file guaranteed to sit at the package root.
PKG="$(dirname "$(find "$HERE/out" -name '_table.pywinrt.txt' | head -1)")"
[ -n "$PKG" ] || { echo "no _table.pywinrt.txt under $HERE/out" >&2; exit 1; }

# PyWinRT emits the projection table as text; the runtime loads a binary one.
"$PY_EXE" -m winrt.table "$PKG/_table.pywinrt.txt" "$PKG/_table.pywinrt"

cp "$REPO/target/debug/winasync.dll" "$PKG/"
cp "$REPO/target/debug/winasync.manifest" "$PKG/"
echo "staged: $PKG"
