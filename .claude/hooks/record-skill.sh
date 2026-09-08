#!/bin/bash
# PostToolUse/Skill: remember which skills this session has loaded.
#
# The companion PreToolUse hook (require-skill.sh) reads this state to check
# that the skills CLAUDE.md marks mandatory for a file type were loaded before
# that file is edited. State is per session id, under /tmp, and disposable —
# a missing file just means "nothing loaded yet".

input=$(cat)
session=$(printf '%s' "$input" | jq -r '.session_id // ""')
skill=$(printf '%s' "$input" | jq -r '.tool_input.skill // ""')
# A qualified name (`plugin:bevy`, `apps/web:deploy`) is recorded bare: the
# readers (require-skill.sh, guard-bash.sh) match the bare name exactly.
skill="${skill##*:}"

[ -z "$session" ] && exit 0
[ -z "$skill" ] && exit 0

dir="/tmp/claude-skill-gate"
mkdir -p "$dir" 2>/dev/null || exit 0
printf '%s\n' "$skill" >>"$dir/$session" 2>/dev/null

exit 0
