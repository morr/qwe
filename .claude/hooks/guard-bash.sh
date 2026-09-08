#!/bin/bash
# PreToolUse/Bash: deny a command that breaks a CLAUDE.md rule, or whose rule
# lives in a skill this session has not loaded.
#
# CLAUDE.md carries these as prose: the `git` skill is mandatory before any git
# operation, the `live-app` skill before running the app, cargo always in the
# background and never through a pipe that hides its progress, no
# `--features bevy/dynamic_linking`. Prose holds while the rule is what the
# session is about and slips when the command shows up sideways — a quick
# `cargo test` in the middle of a refactor, a commit at the end of a long
# session that never loaded `git`. This hook is the net, the same trade
# minisklad's copy makes for its brand databases and test runner.
#
# Fails open on anything unexpected — no command, unreadable state: the gate is
# a nudge, never a reason for the session to get stuck.

input=$(cat)
session=$(printf '%s' "$input" | jq -r '.session_id // ""')
cmd=$(printf '%s' "$input" | jq -r '.tool_input.command // ""')
background=$(printf '%s' "$input" | jq -r '.tool_input.run_in_background // false')

[ -z "$cmd" ] && exit 0

state="/tmp/claude-skill-gate/$session"
# No session id — no skill state exists at all; treat everything as loaded, or the
# deny is one nothing can lift: loading a skill in such a session records nothing
# (record-skill.sh exits on an empty session), so the advice could never be followed.
loaded() { [ -z "$session" ] || grep -qxF "$1" "$state" 2>/dev/null; }

# Command position: line start, after ; & | ( or $( — not any command that
# merely mentions the word (`grep cargo`, `echo git`).
cmd_pos='(^|[;&|(]|\$\()[[:space:]]*'

reason=""

# --- git: mutating subcommands need the `git` skill (hard rules + message style).
# Reads — status, log, diff, show, rev-parse, ls-files, branch/worktree listings,
# fetch — stay open: the skill's rules are about what lands in history.
git_mutating='(add|commit|rebase|merge|checkout|switch|reset|push|pull|stash|cherry-pick|revert|restore|rm|mv|tag|am|apply|update-index|clean|notes)'
if printf '%s' "$cmd" | grep -qE "${cmd_pos}git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?(-c[[:space:]]+[^[:space:]]+[[:space:]]+)?${git_mutating}([[:space:]]|$)" ||
   printf '%s' "$cmd" | grep -qE "${cmd_pos}git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?worktree[[:space:]]+(add|remove|prune|move|lock|unlock)([[:space:]]|$)" ||
   printf '%s' "$cmd" | grep -qE "${cmd_pos}git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?branch[[:space:]]+.*(-[dDmMcCu]([[:space:]]|$)|--delete|--move|--copy|--set-upstream|--unset-upstream|--force)"; then
  if ! loaded git; then
    reason="This is a git operation and the \`git\` skill is not loaded in this session — CLAUDE.md marks it mandatory before any git operation (hard rules: no Claude trailer, stage by name, no --amend, no --no-verify; the commit message style). Call the Skill tool for \`git\`, then repeat this command."
  fi
fi

# --- cargo: the four heavy commands (and tools/check.sh, which wraps them) run
# in the background — a cold build takes minutes and a foreground call blocks
# the session with nothing on screen.
cargo_heavy="${cmd_pos}cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?(build|run|test|clippy|check|bench|doc|nextest)([[:space:]]|$)"
is_cargo=false
if printf '%s' "$cmd" | grep -qE "$cargo_heavy" || printf '%s' "$cmd" | grep -qE "${cmd_pos}([^[:space:]]*/)?tools/check\.sh([[:space:]]|$)"; then
  is_cargo=true
fi

if [ -z "$reason" ] && [ "$is_cargo" = true ]; then
  case "$cmd" in
    *--features*bevy/dynamic_linking*|*--features=*bevy/dynamic_linking*)
      reason="\`dynamic_linking\` is already enabled in Cargo.toml — never pass \`--features bevy/dynamic_linking\` (CLAUDE.md, Build & Development Commands). Drop the flag and repeat." ;;
  esac
fi

if [ -z "$reason" ] && [ "$is_cargo" = true ] &&
   printf '%s' "$cmd" | grep -qE "(cargo[[:space:]]+[^|]*|tools/check\.sh[^|]*)\|[[:space:]]*([^[:space:]]*/)?(tail|head|grep|rg|less|more)([[:space:]]|$)"; then
  reason="Never pipe cargo through tail/head/grep — the pipe buffers everything until the process ends, so no progress is visible, and cargo drops its progress bar when stderr is not a tty. Run it in the background with run_in_background: true (use --message-format=short if the output must be smaller) and read the tail of the finished task output instead."
fi

if [ -z "$reason" ] && [ "$is_cargo" = true ] && [ "$background" != "true" ]; then
  reason="Run cargo build / run / test / clippy (and tools/check.sh) with run_in_background: true — a cold build here takes minutes and a foreground call blocks the session with nothing on screen (CLAUDE.md, Running cargo so progress stays visible). Repeat this exact command with run_in_background: true."
fi

# --- the live app: `cargo run` of the app itself and the `brp` CLI need the
# `live-app` skill (ready markers, SimSpeed vs Time<Virtual>, screenshots,
# shutdown). `cargo run --example …` is a headless yard, not the app.
if [ -z "$reason" ]; then
  is_app_run=false
  if printf '%s' "$cmd" | grep -qE "${cmd_pos}cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?run([[:space:]]|$)" &&
     ! printf '%s' "$cmd" | grep -qE -- '--example([[:space:]=]|$)'; then
    is_app_run=true
  fi
  if printf '%s' "$cmd" | grep -qE "${cmd_pos}([^[:space:]]*/)?brp([[:space:]]|$)"; then
    is_app_run=true
  fi
  if [ "$is_app_run" = true ] && ! loaded live-app; then
    reason="This runs or drives the live app and the \`live-app\` skill is not loaded in this session — CLAUDE.md marks it mandatory before running the app or querying it over BRP (launch in the background, ready markers, the brp CLI, screenshots, shutting it down; this project's inventory is .claude/live-app-project.md). Call the Skill tool for \`live-app\`, then repeat this command."
  fi
fi

[ -z "$reason" ] && exit 0

jq -n --arg reason "$reason" '{
  hookSpecificOutput: {
    hookEventName: "PreToolUse",
    permissionDecision: "deny",
    permissionDecisionReason: $reason
  }
}'
