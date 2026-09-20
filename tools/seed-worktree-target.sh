#!/bin/bash
# Seed a build directory from the main checkout's target/, so cargo rebuilds
# only the path packages (qwe, vendor/polyanya) and treats every registry
# crate — bevy included — as fresh.
#
#   tools/seed-worktree-target.sh [target-dir]
#
# With no argument it seeds `target/` next to the current directory's checkout.
# Pass a path to seed a build directory somewhere else — a `CARGO_TARGET_DIR`
# you are about to use, in a worktree or anywhere on this machine.
#
# Why this works, and why a shared CARGO_TARGET_DIR does not:
#
# A registry crate's fingerprint keys on its source path under
# ~/.cargo/registry, which is the same for every worktree. Copy those artifacts
# into a target/ of this checkout's own and cargo finds them fresh. The two
# packages whose sources live in the repo — qwe and the vendored polyanya — have
# a different path per worktree, so they rebuild here, into this target/.
# Nothing is shared at build time, so nothing can clobber the main checkout's
# libqwe the way one shared CARGO_TARGET_DIR did.
#
# Measured on a 51 GB / 91 667-file target: the clone took 20.7 s and cost zero
# disk; the build that followed compiled polyanya and qwe only, finished in
# 1m30s, and the two copies diverged by 4 GB. On an 88 GB target the clone took
# 50 s, still at zero disk. Without the seed that same worktree builds bevy from
# scratch — measured at 7m29s for 406 crates.
#
# On APFS `cp -c` is a copy-on-write clone. On any other volume it would be a
# real 51 GB copy, so the flag is clonefile-only and the failure path below
# removes the partial tree rather than letting it fall back.
#
# Exit 0 when the destination is ready to build in (seeded now, or already
# populated, or there is nothing to seed from); exit 1 when the clone was
# refused, with the reason on stderr.
set -u

dest="${1:-}"

# The main checkout owns the target/ everything is seeded from: --git-common-dir
# resolves to <main>/.git from a worktree as well as from the main checkout.
common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || {
  echo "seed-worktree-target: not inside a git repository" >&2
  exit 1
}
root=$(dirname "$common")

if [ -z "$dest" ]; then
  top=$(git rev-parse --show-toplevel 2>/dev/null) || exit 1
  dest="$top/target"
fi
case "$dest" in
  /*) ;;
  *) dest="$PWD/$dest" ;;
esac

if [ "$dest" = "$root/target" ]; then
  echo "seed-worktree-target: $dest is the main checkout's own target — nothing to seed" >&2
  exit 0
fi

# A populated destination is left alone: cargo's own freshness check is finer
# than anything this script could decide, and re-cloning over it would be a
# real copy, not a clone.
if [ -d "$dest" ] && [ -n "$(ls -A "$dest" 2>/dev/null)" ]; then
  echo "seed-worktree-target: $dest already exists — left as it is" >&2
  exit 0
fi

if [ ! -d "$root/target" ]; then
  echo "seed-worktree-target: the main checkout has no target/ to seed from — this build starts from scratch" >&2
  exit 0
fi

# A build running in the main checkout would be copied half-written.
if pgrep -f '^(rustc|cargo) ' >/dev/null 2>&1; then
  echo "seed-worktree-target: a cargo/rustc process is running — not seeding from a moving build; wait for it to finish and run this again" >&2
  exit 1
fi

started=$(date +%s)
mkdir -p "$(dirname "$dest")"
# -c demands clonefile and fails rather than falling back to a real copy.
if cp -Rc "$root/target" "$dest" 2>/dev/null; then
  echo "seed-worktree-target: cloned $root/target -> $dest in $(($(date +%s) - started))s (copy-on-write, no disk used) — cargo rebuilds only qwe and vendor/polyanya" >&2
  exit 0
fi

rm -rf "$dest"
echo "seed-worktree-target: clonefile refused (not APFS?) — $dest left empty, a build there compiles bevy from scratch" >&2
exit 1
