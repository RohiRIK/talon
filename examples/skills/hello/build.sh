#!/usr/bin/env bash
# Build the hello skill to a wasm32-wasip2 component and refresh the committed
# test fixture used by talon-plugins integration tests.
#
# WHY THIS SCRIPT EXISTS — the toolchain gotcha:
# This machine has TWO Rust installs. Homebrew rust (/opt/homebrew/bin, first on
# PATH) builds the main workspace but has NO wasm std. The wasm32-wasip2 std
# lives only in the rustup-managed `stable` toolchain. If cargo picks Homebrew
# rustc off PATH it fails with "can't find crate for core/std". So we resolve the
# rustup rustc explicitly and pin RUSTC + PATH to it for this build only. The
# main workspace is untouched and keeps building on Homebrew rust.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fixture="$here/../../../crates/talon-plugins/tests/fixtures/hello.wasm"

# Resolve the rustup `stable` toolchain (the one with the wasm32-wasip2 target).
if ! command -v rustup >/dev/null 2>&1; then
    echo "error: rustup not found. Install it: brew install rustup && rustup-init -y" >&2
    exit 1
fi
rustc_bin="$(rustup which --toolchain stable rustc)"
tc_bin="$(dirname "$rustc_bin")"

rustup target add --toolchain stable wasm32-wasip2 >/dev/null 2>&1 || true

echo "building hello skill with $("$rustc_bin" --version)"
env PATH="$tc_bin:$PATH" RUSTC="$rustc_bin" \
    "$tc_bin/cargo" build --release --target wasm32-wasip2 --manifest-path "$here/Cargo.toml"

out="$here/target/wasm32-wasip2/release/hello_skill.wasm"
mkdir -p "$(dirname "$fixture")"
cp "$out" "$fixture"
echo "fixture updated: $fixture ($(wc -c < "$fixture") bytes)"
