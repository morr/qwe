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
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')
background=$(printf '%s' "$input" | jq -r '.tool_input.run_in_background // false')

[ -z "$cmd" ] && exit 0

state="/tmp/claude-skill-gate/$session"
# No session id — no skill state exists at all; treat everything as loaded, or the
# deny is one nothing can lift: loading a skill in such a session records nothing
# (record-skill.sh exits on an empty session), so the advice could never be followed.
loaded() { [ -z "$session" ] || grep -qxF "$1" "$state" 2>/dev/null; }

# Command position: line start, after ; & | ( or $( — not any command that
# merely mentions the word (`grep cargo`, `echo git`) — and past any leading
# environment assignments or a wrapper with its options
# (`CARGO_TARGET_DIR=/x cargo build`, `CARGO_INCREMENTAL=0 cargo test`,
# `time cargo build`, `nice -n 10 cargo test`, `sudo -u me git commit`): the
# word after those is still the command. An assignment value with spaces
# inside quotes is not covered.
cmd_prefix='([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+|(env|time|nice|sudo|command|exec|nohup)([[:space:]]+[^[:space:];&|]+)*[[:space:]]+)*'
cmd_pos='(^|[;&|(]|\$\()[[:space:]]*'"$cmd_prefix"

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

# --- cargo: the heavy commands — the four CLAUDE.md names (build, run, test,
# clippy) plus check, bench, doc and nextest, which compile just the same — and
# tools/check.sh, which wraps them, run in the background: a cold build takes
# minutes and a foreground call blocks the session with nothing on screen.
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
  reason="Run cargo build / run / test / clippy / check / bench / doc / nextest (and tools/check.sh) with run_in_background: true — a cold build here takes minutes and a foreground call blocks the session with nothing on screen (CLAUDE.md, Running cargo so progress stays visible). Repeat this exact command with run_in_background: true."
fi

# --- the live app: `cargo run` of the app itself and the `brp` CLI need the
# `live-app` skill (ready markers, SimSpeed vs Time<Virtual>, screenshots,
# shutdown). `cargo run --example …` is a headless yard, not the app — but the
# exemption holds only for the `cargo run` that carries the flag, so the
# command is split at ; & | and each segment is tested on its own
# (`cargo run --example x && cargo run` still runs the app).
if [ -z "$reason" ]; then
  is_app_run=false
  cargo_run="${cmd_pos}cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?run([[:space:]]|$)"
  while IFS= read -r segment; do
    if printf '%s' "$segment" | grep -qE "$cargo_run" &&
       ! printf '%s' "$segment" | grep -qE -- '--example([[:space:]=]|$)'; then
      is_app_run=true
    fi
  done <<<"$(printf '%s' "$cmd" | tr ';&|' '\n')"
  if printf '%s' "$cmd" | grep -qE "${cmd_pos}([^[:space:]]*/)?brp([[:space:]]|$)"; then
    is_app_run=true
  fi
  if [ "$is_app_run" = true ] && ! loaded live-app; then
    reason="This runs or drives the live app and the \`live-app\` skill is not loaded in this session — CLAUDE.md marks it mandatory before running the app or querying it over BRP (launch in the background, ready markers, the brp CLI, screenshots, shutting it down; this project's inventory is .claude/live-app-project.md). Call the Skill tool for \`live-app\`, then repeat this command."
  fi
fi

# --- source files are edited with Edit/Write, never rewritten from a script.
# The rule is CLAUDE.md's ("Editing files — use Edit, not a Python script"), and
# it is the one that slips hardest: a session told to prefer Bash drifts into
# `python3 - <<PY … write_text()` call by call, and a `str.replace()` that
# matches nothing is a silent no-op. It also silently defeats the *other* gate —
# require-skill.sh only sees Edit|Write, so a file rewritten from Bash skips the
# domain-skill check entirely. One measured session made 154 such writes (132 of
# them .rs) against 124 Edits, and the gate fired on none of them.
#
# The exemption CLAUDE.md grants — a genuinely mechanical sweep across many call
# sites, where Edit has no equivalent — is spelled, not guessed: the command
# carries the literal marker `# mechanical-sweep`.
if [ -z "$reason" ] && ! printf '%s' "$cmd" | grep -qF '# mechanical-sweep'; then
  # Bound forms first — the target is the token the construct writes to, so a
  # `grep src/x.rs > /tmp/out` is not mistaken for a rewrite of src/x.rs.
  targets=$(
    printf '%s' "$cmd" | grep -oE '(^|[^0-9<>])>>?[[:space:]]*[^[:space:];&|)"'"'"'<>]+' |
      sed -E 's/.*>>?[[:space:]]*//'
    printf '%s' "$cmd" | grep -oE '\btee[[:space:]]+(-a[[:space:]]+)?[^[:space:];&|)"'"'"'<>]+' |
      sed -E 's/^tee[[:space:]]+(-a[[:space:]]+)?//'
  )
  # Unbound forms — an in-place editor or an interpreter writing a file. Their
  # target is inside the script, so every path-looking token counts.
  if printf '%s' "$cmd" | grep -qE '(sed|perl)[[:space:]]+[^|;&]*(-[A-Za-z]*i|--in-place)|write_text\(|writeFileSync|File\.write|\.write\(|open\([^)]*["'"'"'][wa]'; then
    targets="$targets
$(printf '%s' "$cmd" | grep -oE '[A-Za-z0-9_./-]+\.(rs|wgsl|md|toml)')"
  fi

  root="${CLAUDE_PROJECT_DIR:-$cwd}"
  for target in $targets; do
    case "$target" in
      "$root"/*) target="${target#"$root"/}" ;;
      /*|~*) continue ;;   # outside the project (scratchpad, /tmp, a sibling repo)
    esac
    case "$target" in
      .claude/worktrees/*/*) target="${target#.claude/worktrees/*/}" ;;
    esac
    case "$target" in
      src/*.rs|tests/*.rs|examples/*.rs|benches/*.rs|assets/*.wgsl|*.md|Cargo.toml)
        reason="Edit and Write are how source files change here — never a script that rewrites one (CLAUDE.md, \"Editing files — use Edit, not a Python script\"). This command writes \`$target\`: a \`str.replace()\` that matches nothing is a silent no-op, and a file rewritten from Bash also skips the domain-skill gate, which only watches Edit|Write. Grep the anchor, Read a window around it, then Edit against a line you have just seen. If this really is a mechanical sweep across many call sites — the one case a script wins — say so by putting the literal \`# mechanical-sweep\` in the command, assert on every replacement, and \`git diff\` afterwards."
        break ;;
    esac
  done
fi

[ -z "$reason" ] && exit 0

jq -n --arg reason "$reason" '{
  hookSpecificOutput: {
    hookEventName: "PreToolUse",
    permissionDecision: "deny",
    permissionDecisionReason: $reason
  }
}'
