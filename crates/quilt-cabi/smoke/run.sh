#!/usr/bin/env sh
# Build the quilt-cabi cdylib, regenerate the golden fixtures from
# compat/golden.json, compile the C smoke harness against the real .so,
# and run it. Usage: smoke/run.sh [release]   (default: debug)
set -eu
cd "$(dirname "$0")/../../.."   # repo root (from crates/quilt-cabi/smoke)

# Fixtures are generated, never hand-copied. (Falls back to the checked-in
# copies if python3 is unavailable.)
if command -v python3 >/dev/null 2>&1; then
    python3 crates/quilt-cabi/smoke/gen-sheet.py
fi

if [ "${1:-}" = "release" ]; then
    cargo build -p quilt-cabi --release
    OUT=target/release
else
    cargo build -p quilt-cabi
    OUT=target/debug
fi

for lib in "$OUT/libquilt_cabi.so" "$OUT/libquilt_cabi.a"; do
    if [ ! -f "$lib" ]; then
        echo "missing artifact: $lib" >&2
        exit 1
    fi
done
echo "artifacts: $OUT/libquilt_cabi.so + $OUT/libquilt_cabi.a"

cc -std=c11 -Wall -Wextra -Icrates/quilt-cabi -Icrates/quilt-cabi/smoke \
   crates/quilt-cabi/smoke/smoke.c \
   -o "$OUT/quilt_cabi_smoke" \
   -L"$OUT" -lquilt_cabi "-Wl,-rpath,$PWD/$OUT"

"$OUT/quilt_cabi_smoke"
