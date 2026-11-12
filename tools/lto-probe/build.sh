#!/usr/bin/env bash
# Builds the probe two ways and shows what became of the vtable call.
#
#   native  Rust and C++ compiled to machine code, linked as usual.
#   lto     both compiled to LLVM bitcode (ThinLTO), linked by rustc's own
#           lld, which runs the LTO over the whole program.
#   fat     the same with full LTO on both sides.
#
# rustc's LLVM is one major ahead of the installed clang (23 vs 22), so the
# link must be done by rust-lld: a newer lld reads older bitcode, not the
# other way round. clang-cl is used only to compile.
set -euo pipefail
cd "$(dirname "$0")"

CLANG="/c/Program Files/LLVM/bin/clang-cl.exe"
OBJDUMP="/c/Program Files/LLVM/bin/llvm-objdump.exe"
SYSROOT="$(rustc --print sysroot)"
RUSTLLD="$SYSROOT/lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe"
MSVC_LIB="C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/MSVC/14.44.35207/lib/x64"
SDK="C:/Program Files (x86)/Windows Kits/10/Lib/10.0.26100.0"
export LIB="$MSVC_LIB;$SDK/um/x64;$SDK/ucrt/x64"
MSVC_INC="C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/MSVC/14.44.35207/include"
SDK_INC="C:/Program Files (x86)/Windows Kits/10/Include/10.0.26100.0"
export INCLUDE="$MSVC_INC;$SDK_INC/ucrt;$SDK_INC/um;$SDK_INC/shared"
# What `--print native-static-libs` lists for the staticlib.
LIBS="kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib"

OUT=out
mkdir -p "$OUT"
# lld picks its flavor from its own file name.
cp "$RUSTLLD" "$OUT/lld-link.exe"

echo "== native"
cargo build --release -q --target-dir target/native
"$CLANG" -nologo -O2 -MD -EHsc -c consumer.cpp "-Fo$OUT/consumer_native.obj"
"$OUT/lld-link.exe" -nologo "-out:$OUT/probe_native.exe" "$OUT/consumer_native.obj" \
    target/native/release/lto_probe.lib $LIBS -subsystem:console

echo "== lto"
# An explicit --target keeps the flags off the host build scripts and
# proc-macros, which link.exe links and which cannot be bitcode.
RUSTFLAGS="-Clinker-plugin-lto -Cembed-bitcode=yes" cargo build --release -q --target-dir target/lto --target x86_64-pc-windows-msvc
# ThinLTO on both sides: rustc's bitcode carries ThinLTO summaries, and a
# full-LTO object would land in a separate partition with no inlining
# across. The CPU level matters too: the inliner refuses a callee whose
# target features the caller lacks, and rustc's x86-64 baseline turns on
# cx16, sse3 and sahf that clang's does not ("conflicting target
# features" in the inline remarks). x86-64-v2 is a superset of both.
"$CLANG" -nologo -O2 -MD -EHsc -flto=thin /clang:-march=x86-64-v2 -c consumer.cpp "-Fo$OUT/consumer_lto.obj"
"$OUT/lld-link.exe" -nologo "-out:$OUT/probe_lto.exe" "$OUT/consumer_lto.obj" \
    target/lto/x86_64-pc-windows-msvc/release/lto_probe.lib $LIBS -subsystem:console -opt:lldlto=3 \
    "-mllvm:-pass-remarks=inline" "-mllvm:-pass-remarks-missed=inline" 2>&1 \
    | grep -E "'probe_" | sed -E 's/^<unknown>:0:0: //' | sort -u

echo "== fat"
# Full LTO on both sides, one module: the only mode in which a call
# through the exported vtable symbol inlines. Under ThinLTO the slot load
# folds to a direct call, but the thunk is never imported (imports follow
# call edges, and a vtable is a reference, not a call).
RUSTFLAGS="-Clinker-plugin-lto -Clto=fat -Cembed-bitcode=yes" cargo build --release -q --target-dir target/lto_fat --target x86_64-pc-windows-msvc
"$CLANG" -nologo -O2 -MD -EHsc -flto /clang:-march=x86-64-v2 -c consumer.cpp "-Fo$OUT/consumer_fat.obj"
"$OUT/lld-link.exe" -nologo "-out:$OUT/probe_fat.exe" "$OUT/consumer_fat.obj" \
    target/lto_fat/x86_64-pc-windows-msvc/release/lto_probe.lib $LIBS -subsystem:console -opt:lldlto=3 \
    "-mllvm:-pass-remarks=inline" 2>&1 \
    | grep -E "'probe_|Vtbl3new" | sed -E 's/^<unknown>:0:0: //' | sort -u

disassemble() { # exe, rva, label
    local start=$((0x140000000 + 0x$2))
    echo "--- $3 @ $2"
    "$OBJDUMP" -d --x86-asm-syntax=intel --no-show-raw-insn \
        --start-address=$start --stop-address=$((start + 0xa0)) "$1" | tail -n +6 | grep -E "call|ret" || true
}

for v in native lto fat; do
    echo "== run $v"
    # -a: with stderr redirected to a file, reopening it without append
    # would truncate the log so far.
    line=$("$OUT/probe_$v.exe" 7 200000000 | tee -a /dev/stderr | grep '^rva')
    rv=$(sed -E 's/.*run_vtable=([0-9a-f]+).*/\1/' <<<"$line")
    rd=$(sed -E 's/.*run_direct=([0-9a-f]+).*/\1/' <<<"$line")
    rf=$(sed -E 's/.*run_vtable_fresh=([0-9a-f]+).*/\1/' <<<"$line")
    rh=$(sed -E 's/.*run_vtable_hoisted=([0-9a-f]+).*/\1/' <<<"$line")
    rs=$(sed -E 's/.*run_vtable_symbol=([0-9a-f]+).*/\1/' <<<"$line")
    disassemble "$OUT/probe_$v.exe" "$rv" "run_vtable ($v)"
    disassemble "$OUT/probe_$v.exe" "$rd" "run_direct ($v)"
    disassemble "$OUT/probe_$v.exe" "$rf" "run_vtable_fresh ($v)"
    disassemble "$OUT/probe_$v.exe" "$rh" "run_vtable_hoisted ($v)"
    disassemble "$OUT/probe_$v.exe" "$rs" "run_vtable_symbol ($v)"
done
