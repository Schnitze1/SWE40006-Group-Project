#!/bin/bash
# Run inside amazonlinux:2023 (glibc 2.34) — matches Lambda provided.al2023.
set -euo pipefail
cd /work/pii-tool

dnf install -y gcc gcc-c++ make zip unzip openssl-devel git cmake pkgconf perl ca-certificates findutils tar patchelf

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
source "$HOME/.cargo/env"

cc -c crates/lambda/glibc_compat.c -o /tmp/glibc_compat.o
RUSTFLAGS='-C link-arg=/tmp/glibc_compat.o' cargo build --release -p poco-lambda

OUT=target/lambda/poco-lambda
rm -rf "$OUT"
mkdir -p "$OUT"
cp target/release/poco-lambda "$OUT/bootstrap"
chmod +x "$OUT/bootstrap"

# Belt and braces: strip any leftover GLIBC_2.38 version refs if present.
patchelf --clear-symbol-version __isoc23_strtoll "$OUT/bootstrap" || true
patchelf --clear-symbol-version __isoc23_strtol "$OUT/bootstrap" || true
patchelf --clear-symbol-version __isoc23_strtoull "$OUT/bootstrap" || true
patchelf --clear-symbol-version __isoc23_strtoul "$OUT/bootstrap" || true

echo '=== binary ==='
file "$OUT/bootstrap"
echo '=== GLIBC_2.38 must be none ==='
objdump -T "$OUT/bootstrap" | grep GLIBC_2.38 || echo none
echo '=== ldd ==='
ldd "$OUT/bootstrap" || true

(cd "$OUT" && zip -q bootstrap.zip bootstrap)
echo '=== zip ==='
unzip -l "$OUT/bootstrap.zip"
test -f "$OUT/bootstrap.zip"
