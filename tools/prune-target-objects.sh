#!/bin/bash
# Delete the orphaned incremental object files cargo leaves in
# <target>/*/deps and <target>/*/examples.
#
#   tools/prune-target-objects.sh [target-dir]
#
# With no argument it prunes `$CARGO_TARGET_DIR`, or `target/` of this checkout.
# tools/check.sh runs it after every check; run it by hand after a day of plain
# `cargo test` / `cargo run`.
#
# Why they pile up. On macOS the dev profile's debuginfo is `unpacked`: the
# binary carries only a debug map, and the DWARF stays in the codegen units'
# `.o` files, which have to outlive the build. With incremental compilation
# rustc keeps each CGU's `.o` in the session directory under incremental/ and
# hard-links it into deps/ under a name with a fresh per-session suffix —
# `qwe-<hash>.<cgu>.<session>.rcgu.o`. When the next session replaces that
# directory, the deps/ link is left behind with nobody pointing at it; nothing
# in cargo or rustc ever deletes it. Every rebuild of every path-package target
# (the lib, its unit tests, the bin, each tests/*.rs, each example) leaves one
# more full set. Measured here: 77 506 such files, 127 GB of a 133 GB deps/,
# the same CGU present 106 times over.
#
# Which ones are safe. A live object is still linked from its session
# directory, so its link count is 2 or more; an orphan's is 1. The name pattern
# matches only incremental objects — registry crates are built once,
# non-incrementally, as `<crate>-<hash>.<crate>.<hash>-cgu.NN.rcgu.o`, and their
# single-linked objects are the only DWARF those crates have, so they are never
# touched. A binary linked from an orphaned set loses line numbers in its
# backtraces, but by then cargo has relinked it against the new set, which is
# exactly what the link count reports.
#
# A freshly seeded worktree (tools/seed-worktree-target.sh) is the one place
# where every object has a link count of 1: a clonefile copy does not carry
# hard links over. Pruning there deletes the main checkout's objects that were
# cloned along — which the worktree never uses, since its own binaries relink
# against the objects of its own build.
set -u

target="${1:-${CARGO_TARGET_DIR:-$(cd "$(dirname "$0")/.." && pwd)/target}}"
[ -d "$target" ] || exit 0

# Examples link their objects from examples/, not deps/ — same naming, same leak.
for deps in "$target"/*/deps "$target"/*/examples; do
    [ -d "$deps" ] || continue
    list=$(mktemp)
    find -E "$deps" -maxdepth 1 -type f -links 1 \
        -regex '.*/[A-Za-z0-9_]+-[0-9a-f]{16}\.[0-9a-z]+\.[0-9a-z]+\.rcgu\.o' \
        -print0 >"$list"
    count=$(tr -cd '\0' <"$list" | wc -c | tr -d ' ')
    if [ "$count" -gt 0 ]; then
        bytes=$(xargs -0 stat -f '%z' <"$list" | awk '{s += $1} END {printf "%.2f", s / 1e9}')
        xargs -0 rm -f <"$list"
        echo "prune-target-objects: $deps — removed $count orphaned objects, $bytes GB"
    fi
    rm -f "$list"
done
