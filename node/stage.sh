#!/usr/bin/env bash
# Builds the component and stages a Node projection of it under out/.
#
# Everything under out/ is generated, so nothing it produces is checked in.
# Run it once after `npm install`, and again after `cargo run -p winmdgen`
# changes the metadata.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
NAMESPACE="Pbk20191.WinAsync.Comio"

# The server DLL, and the reg-free manifest the activation test writes out
# of `activation::manifest()` beside it -- as for python/.
(cd "$REPO" && cargo build -p winasync-dll && cargo test --features comio --test activation)

rm -rf "$HERE/out"
mkdir -p "$HERE/out"

# Two generated packages. Ours, from the published metadata -- relative
# paths, because `$REPO` is a Git Bash path the tool cannot resolve -- and
# the two system classes the script reads and writes with. dynwinrt
# generates nothing for `Windows.*` unless asked by name.
(cd "$HERE" && npx --no-install dynwinrt-codegen generate \
    --winmd "../metadata/$NAMESPACE.winmd" \
    --namespace "$NAMESPACE" \
    --output out/comio)
(cd "$HERE" && npx --no-install dynwinrt-codegen generate \
    --namespace Windows.Storage.Streams \
    --class-name DataWriter,DataReader \
    --output out/streams)

# `<file name="winasync.dll">` in the manifest has no path: it resolves
# against the manifest's own directory.
cp "$REPO/target/debug/winasync.dll" "$HERE/out/"
cp "$REPO/target/debug/winasync.manifest" "$HERE/out/"
echo "staged: $HERE/out"
