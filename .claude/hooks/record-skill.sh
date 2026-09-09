#!/bin/bash
# PostToolUse/Skill: remember which skills this session has loaded.
#
# The companion PreToolUse hook (require-skill.sh) reads this state to check
# that the skills CLAUDE.md marks mandatory for a file type were loaded before
# that file is edited. State is per session id, under /tmp, and disposable —
# a missing file just means "nothing loaded yet".
#
# Per session id is also per *context*: a session continued after compaction
# gets a new id and starts with an empty record, which is the honest answer —
# the skill's text was truncated away with the rest of the old context, so it
# has to be loaded again anyway. See CLAUDE.md ("after a compaction").

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

# One file per session and nothing ever deletes them: sweep the ones no live
# session can own any more. A week is far past any session's life, and losing
# one only costs a re-load.
find "$dir" -type f -mtime +7 -delete 2>/dev/null

exit 0
