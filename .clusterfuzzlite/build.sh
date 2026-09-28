#!/bin/bash -eu
# Build script for ClusterFuzzLite. Runs inside base-builder-rust, which ships
# a nightly toolchain and cargo-fuzz preinstalled.
#
# --debug-assertions keeps integer-overflow checks on in the optimized build;
# without it, crashes like `attempt to add with overflow` (the class found by
# css_scope_no_panic) are silently compiled away.

cd "$SRC/compiler-rs"
cargo fuzz build -O --debug-assertions

# The fuzz crate is its own workspace, so binaries land under fuzz/target/.
FUZZ_TARGET_OUTPUT_DIR="fuzz/target/x86_64-unknown-linux-gnu/release"
for f in fuzz/fuzz_targets/*.rs; do
  FUZZ_TARGET_NAME="$(basename "${f%.*}")"
  cp "$FUZZ_TARGET_OUTPUT_DIR/$FUZZ_TARGET_NAME" "$OUT/"
done
