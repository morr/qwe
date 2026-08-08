#!/bin/bash
# One-call verification: build + test + clippy + fmt of the files you touched.
#
#   tools/check.sh [changed-file.rs ...]
#
# Run it in the background (run_in_background: true) like any cargo command —
# one background task instead of four, one tail to read instead of four.
#
# Files passed as arguments are rustfmt-ed (nightly rustfmt, per CLAUDE.md).
# With no arguments fmt is skipped on purpose: never fmt files you did not
# change — the user's parallel WIP may sit unstaged in them.
set -u
cd "$(dirname "$0")/.."

failed=""

echo "== cargo build"
# A broken build fails test and clippy with the same errors — stop here rather
# than print them three times.
cargo build --message-format=short 2>&1 || { echo "check: FAILED (build)"; exit 1; }

echo "== cargo test"
cargo test -q --message-format=short 2>&1 || failed="$failed test"

echo "== cargo clippy"
cargo clippy --all-targets --message-format=short -- -D warnings 2>&1 || failed="$failed clippy"

if [ "$#" -gt 0 ]; then
    echo "== rustfmt $*"
    RUSTFMT=~/.rustup/toolchains/nightly-aarch64-apple-darwin/bin/rustfmt \
        cargo fmt -- "$@" || failed="$failed fmt"
fi

if [ -n "$failed" ]; then
    echo "check: FAILED ($failed)"
    exit 1
fi
echo "check: OK"
