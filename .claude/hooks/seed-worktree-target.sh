#!/bin/bash
# PostToolUse/EnterWorktree: seed the fresh worktree's target/ from the main
# checkout, so cargo rebuilds only qwe and vendor/polyanya instead of bevy and
# 400 registry crates.
#
# The work itself lives in tools/seed-worktree-target.sh — the same script a
# session runs by hand after making a worktree with `git worktree add`, or
# before pointing CARGO_TARGET_DIR at a fresh directory. This hook only fires on
# the EnterWorktree *tool*, so a worktree made from Bash gets no event and no
# seed; guard-bash.sh is what catches that case, and its deny names the script.
#
# Skips silently when the worktree already has a target/, the main checkout has
# none, or the clone fails. The hook seeds a build; it never blocks one.

input=$(cat)
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')

root="${CLAUDE_PROJECT_DIR:-}"
[ -z "$root" ] && exit 0
[ -z "$cwd" ] && exit 0
[ "$cwd" = "$root" ] && exit 0
[ -d "$cwd" ] || exit 0

cd "$root" || exit 0
"$root/tools/seed-worktree-target.sh" "$cwd/target" >&2

exit 0
