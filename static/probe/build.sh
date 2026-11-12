#!/usr/bin/env bash
# Links consumer.cpp against winasync_static.lib with full cross-language
# LTO and shows what the inliner did with IClosable::Close reached through
# the object's vtable and through the class's exported table. The toolchain
# notes are the same as tools/lto-probe/build.sh: rustc's own lld links,
# because its LLVM is newer than the installed clang's; clang-cl only
# compiles; the caller's target features must cover rustc's baseline.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=../..

CLANG="/c/Program Files/LLVM/bin/clang-cl.exe"
SYSROOT="$(rustc --print sysroot)"
RUSTLLD="$SYSROOT/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe"
MSVC="C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/MSVC/14.44.35207"
SDK="C:/Program Files (x86)/Windows Kits/10"
SDKV="10.0.26100.0"
export LIB="$MSVC/lib/x64;$SDK/Lib/$SDKV/um/x64;$SDK/Lib/$SDKV/ucrt/x64"
export INCLUDE="$MSVC/include;$SDK/Include/$SDKV/ucrt;$SDK/Include/$SDKV/um;$SDK/Include/$SDKV/shared;$SDK/Include/$SDKV/winrt"
# `--print native-static-libs` for winasync-static, plus HSTRING's.
LIBS="kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib runtimeobject.lib"

OUT=out
mkdir -p "$OUT"
cp "$RUSTLLD" "$OUT/lld-link.exe"

echo "== library (fat LTO bitcode)"
(cd "$ROOT" && RUSTFLAGS="-Clinker-plugin-lto -Clto=fat -Cembed-bitcode=yes" \
    cargo build -q -p winasync-static --release --target-dir static/probe/target --target x86_64-pc-windows-msvc)
LIBFILE="target/x86_64-pc-windows-msvc/release/winasync_static.lib"

echo "== consumer"
"$CLANG" -nologo -O2 -MD -EHsc -flto /clang:-march=x86-64-v2 -c consumer.cpp "-Fo$OUT/consumer.obj"

echo "== link, with the inliner's remarks for the two Close paths"
"$OUT/lld-link.exe" -nologo "-out:$OUT/probe.exe" "$OUT/consumer.obj" "$LIBFILE" $LIBS \
    -subsystem:console -opt:lldlto=3 "-mllvm:-pass-remarks=inline" "-mllvm:-pass-remarks-missed=inline" 2>&1 \
    | grep -E "close_via" | sed -E 's/^<unknown>:0:0: //' | sort -u || true

echo "== run"
"$OUT/probe.exe"
