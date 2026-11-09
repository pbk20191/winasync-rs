#!/usr/bin/env bash
# Builds the component and stages a Python projection of it beside this script.
#
# Everything here is generated, so nothing it produces is checked in. Run it
# once after `cargo run -p winmdgen` changes the metadata.
#
#   tools:  dotnet tool install --local PyWinRT
#   python: pip install winrt-runtime winrt-Windows.Foundation #                       winrt-Windows.Storage.Streams winrt-table-compiler
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
PY_EXE="${PY_EXE:-python}"
PKG="$HERE/out/rustwinui3loop-Rustwinui3loop/rustwinui3loop/rustwinui3loop"

# The cdylib is what the runtime loads, and the integration test is what
# writes the reg-free manifest out of `activation::manifest()`.
(cd "$REPO" && cargo build --lib --all-features && cargo test --all-features --test activation)

# winmd -> Python projection. `--component-dlls` says the component ships
# inside the package, which is what the copy below arranges.
rm -rf "$HERE/out"
(cd "$HERE" && dotnet pywinrt     --input "rustwinui3loop;$REPO/metadata/rustwinui3loop.winmd"     --reference sdk --output out --include Rustwinui3loop --component-dlls)

# PyWinRT emits the projection table as text; the runtime loads a binary one.
"$PY_EXE" -m winrt.table "$PKG/_table.pywinrt.txt" "$PKG/_table.pywinrt"

cp "$REPO/target/debug/rustwinui3loop.dll" "$PKG/"
cp "$REPO/target/debug/rustwinui3loop.manifest" "$PKG/"
echo "staged: $PKG"
