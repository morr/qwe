#!/bin/bash
# PostToolUse/EnterWorktree: seed a fresh worktree's target/ from the main
# checkout, so cargo rebuilds only the path packages (qwe, vendor/polyanya)
# and treats every registry crate — bevy included — as fresh.
#
# Why this works, and why a shared CARGO_TARGET_DIR does not:
#
# A registry crate's fingerprint keys on its source path under
# ~/.cargo/registry, which is the same for every worktree. Copy those
# artifacts into the worktree's own target/ and cargo finds them fresh. The
# two packages whose sources live in the repo — qwe and the vendored polyanya
# — have a different path per worktree, so they rebuild here, into this
# worktree's own target/. Nothing is shared at build time, so nothing can
# clobber the main checkout's libqwe the way one shared CARGO_TARGET_DIR did.
#
# Measured on a 51 GB / 91 667-file target: the clone took 20.7 s and cost
# zero disk; the build that followed compiled polyanya and qwe only, finished
# in 1m30s, and the two copies diverged by 4 GB. Without the seed that same
# worktree builds bevy from scratch.
#
# On APFS `cp -c` is a copy-on-write clone. On any other volume it would be a
# real 51 GB copy, so the flag is clonefile-only and the failure path below
# removes the partial tree rather than letting it fall back.
#
# Skips silently when: the worktree already has a target/, the main checkout
# has none, or the clone fails. The hook seeds a build; it never blocks one.

input=$(cat)
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')

root="${CLAUDE_PROJECT_DIR:-}"
[ -z "$root" ] && exit 0
[ -z "$cwd" ] && exit 0
[ "$cwd" = "$root" ] && exit 0

case "$cwd" in
  "$root"/.claude/worktrees/*) ;;
  *) exit 0 ;;
esac

[ -d "$cwd/target" ] && exit 0
[ -d "$root/target" ] || exit 0

# A build running in the main checkout would be copied half-written.
if pgrep -f '^(rustc|cargo) ' >/dev/null 2>&1; then
  echo "worktree target: a cargo/rustc process is running — not seeding target/ from a moving build; clone it by hand once that build is done" >&2
  exit 0
fi

started=$(date +%s)
# -c demands clonefile and fails rather than falling back to a real copy.
if cp -Rc "$root/target" "$cwd/target" 2>/dev/null; then
  echo "worktree target: cloned from the main checkout in $(($(date +%s) - started))s (copy-on-write, no disk used) — cargo rebuilds only qwe and vendor/polyanya" >&2
else
  rm -rf "$cwd/target"
  echo "worktree target: clonefile refused (not APFS?) — target/ left empty, this worktree builds bevy from scratch" >&2
fi

exit 0
