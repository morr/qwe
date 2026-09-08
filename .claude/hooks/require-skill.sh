#!/bin/bash
# PreToolUse/Edit|Write: deny an edit whose path has a mandatory skill that
# this session has not loaded.
#
# CLAUDE.md's skill table says which skill to load before touching which
# module. The prose works when the module is what the session is *about*, and
# fails when the module shows up sideways: a UI change that ends up moving a
# navigation threshold, a navigation fix that touches the replay contract. This
# hook closes those misses mechanically — the same net minisklad's copy casts
# over its Ruby/JS/Sass file types.
#
# Fails open on anything unexpected — no session id, no path, unreadable state:
# the gate is a nudge, never a reason for the session to get stuck.

input=$(cat)
session=$(printf '%s' "$input" | jq -r '.session_id // ""')
path=$(printf '%s' "$input" | jq -r '.tool_input.file_path // ""')
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')

[ -z "$session" ] && exit 0
[ -z "$path" ] && exit 0

root="${CLAUDE_PROJECT_DIR:-$cwd}"
[ -z "$root" ] && exit 0
case "$path" in
  "$root"/*) ;;
  *) exit 0 ;;   # outside the project (scratchpad, dotfiles, a sibling repo)
esac

rel="${path#"$root"/}"

# A worktree made by EnterWorktree lives under .claude/worktrees/<name>/ of the
# main checkout, and $CLAUDE_PROJECT_DIR stays the main checkout after entering
# it — strip that prefix first, or the `.claude/*` exemption below would ungate
# every file of the worktree.
case "$rel" in
  .claude/worktrees/*/*) rel="${rel#.claude/worktrees/*/}" ;;
esac

# Never gated: the agent-configuration files themselves and docs.
case "$rel" in
  .claude/*|*.md) exit 0 ;;
esac

required=""
add() { case " $required " in *" $1 "*) ;; *) required="$required $1" ;; esac; }

# Engine level: every Rust file of the app, its tests and its examples is Bevy
# code. vendor/ (polyanya) and tools/ (python) are not.
case "$rel" in
  src/*.rs|tests/*.rs|examples/*.rs) add bevy ;;
esac

# Domain skills, one per row of the CLAUDE.md table. A path may sit in two
# rows (movement/wander.rs, map/osm/download.rs) — both skills are required.
case "$rel" in
  src/map/*) add osm-map ;;
esac
case "$rel" in
  src/navigation/*|src/movement/*|tests/navigation.rs|tests/movement.rs) add navigation-deep ;;
esac
case "$rel" in
  src/rng.rs|src/rng/*|src/determinism/*|tests/determinism.rs|examples/acceptance/*) add determinism ;;
esac
case "$rel" in
  src/human/*|src/demon/*|src/silhouette/*|src/portal.rs|src/movement/wander.rs|src/spatial.rs|tests/spatial.rs) add species-behavior ;;
esac
case "$rel" in
  src/loading.rs|src/restart.rs|src/city.rs|src/map/osm/download.rs) add world-lifecycle ;;
esac
case "$rel" in
  src/sim_time.rs|src/sim_time/*) add sim-speed ;;
esac
case "$rel" in
  src/ui/*|src/camera.rs|src/prefs.rs) add ui-panels ;;
esac

[ -z "$required" ] && exit 0

state="/tmp/claude-skill-gate/$session"
missing=""
for skill in $required; do
  if ! grep -qxF "$skill" "$state" 2>/dev/null; then
    missing="$missing $skill"
  fi
done

[ -z "$missing" ] && exit 0

reason="Load the skill(s) first — CLAUDE.md's skill table marks them required for this path, and they are not loaded in this session:${missing}. Call the Skill tool for each, then repeat this edit. (If one is already loaded via a slash command, calling Skill again is harmless.)"

jq -n --arg reason "$reason" '{
  hookSpecificOutput: {
    hookEventName: "PreToolUse",
    permissionDecision: "deny",
    permissionDecisionReason: $reason
  }
}'

exit 0
