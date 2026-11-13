#!/usr/bin/env bash
# Build the browser engine and generate its JS glue into web/src/wasm/pkg.
set -euo pipefail
cd "$(dirname "$0")/.."
# Panic locations name files: no one's home directory in the download.
export RUSTFLAGS="--remap-path-prefix=$(rustc --print sysroot)/lib/rustlib/src/rust=rust --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}/registry/src=cargo"
cargo build -p tradoor-wasm --target wasm32-unknown-unknown --profile wasm
wasm-bindgen --target web --remove-producers-section --out-dir web/src/wasm/pkg target/wasm32-unknown-unknown/wasm/tradoor_wasm.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --converge --strip-target-features --strip-producers --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
    -o web/src/wasm/pkg/tradoor_wasm_bg.wasm web/src/wasm/pkg/tradoor_wasm_bg.wasm
fi
# Formulas, a module of their own loaded only when a chart has one: no glue, nothing imported.
cargo build -p tradoor-formula-wasm --target wasm32-unknown-unknown --profile wasm
mkdir -p web/src/wasm/formula
cp target/wasm32-unknown-unknown/wasm/tradoor_formula_wasm.wasm web/src/wasm/formula/formula.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --converge --strip-target-features --strip-producers --strip-debug --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
    -o web/src/wasm/formula/formula.wasm web/src/wasm/formula/formula.wasm
fi
for f in web/src/wasm/pkg/tradoor_wasm_bg.wasm web/src/wasm/pkg/tradoor_wasm.js web/src/wasm/formula/formula.wasm; do
  printf '%-45s %8d bytes  %7d gzip\n' "$f" "$(wc -c <"$f")" "$(gzip -9c "$f" | wc -c)"
done
