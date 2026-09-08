#!/bin/bash
# PostToolUse/EnterWorktree|Skill: tell the session what the worktree's branch
# changed under .claude — none of it is what the session is running.
#
# Skills, hooks, agents and settings.json load from $CLAUDE_PROJECT_DIR, the
# directory the session was launched in, and EnterWorktree does not move it
# (verified: after EnterWorktree a Skill call still reports the main checkout as
# its base directory, and the hooks that run are the main checkout's copies).
# Claude Code's own discovery of nested .claude/skills dirs skips gitignored
# ones, and .claude/worktrees is gitignored, so the worktree's copies are never
# picked up on their own. Hence two moments:
#
# - EnterWorktree: report every difference once — the skills the branch changed,
#   the skills that exist only on the branch (they are not in the listing at all,
#   so Skill cannot load them) and the hooks/agents/settings the branch changed
#   (nothing can be done about those in a running session, but they are named).
# - Skill: when the skill just loaded differs on the branch, hand over the
#   branch's SKILL.md, which supersedes the one the tool returned.
#
# A third case is not drift the branch made but drift the worktree makes: a
# skill that is a symlink with a relative target outside the repo
# (.claude/skills/bevy -> ../../../zxc/...) resolves in the main checkout and
# dangles in .claude/worktrees/<name>/. diff -rq swallows that (an unreadable
# path, stderr, exit 2), so it is checked apart and reported in both moments —
# the Skill tool still loads such a skill from the main checkout, but its
# files must be read there, and a session launched inside the worktree would
# not have it at all.
#
# Silent when nothing differs, which is nearly every worktree. Fails open on
# anything unexpected — the hook informs, it never blocks.

input=$(cat)
tool=$(printf '%s' "$input" | jq -r '.tool_name // ""')
cwd=$(printf '%s' "$input" | jq -r '.cwd // ""')

root="${CLAUDE_PROJECT_DIR:-}"
[ -z "$root" ] && exit 0

case "$tool" in
  EnterWorktree) entered=$(printf '%s' "$input" | jq -r '.tool_input.path // ""') ;;
  Skill) entered="" ;;
  *) exit 0 ;;
esac
worktree=$(git -C "${entered:-$cwd}" rev-parse --show-toplevel 2>/dev/null) || exit 0
[ -z "$worktree" ] && exit 0
[ "$worktree" = "$root" ] && exit 0

common=$(git -C "$worktree" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
[ "$common" = "$root/.git" ] || exit 0

emit() {
  jq -n --arg ctx "$1" '{
    hookSpecificOutput: { hookEventName: "PostToolUse", additionalContext: $ctx }
  }'
  exit 0
}

# diff -rq lines, relative to .claude/: "changed <path>", "branch-only <path>",
# "main-only <path>".
claude_diff() {
  diff -rq "$root/.claude/$1" "$worktree/.claude/$1" 2>/dev/null | while IFS= read -r line; do
    case "$line" in
      "Only in $worktree/.claude/"*) rel="${line#"Only in $worktree/.claude/"}"; printf 'branch-only %s\n' "${rel/: //}" ;;
      "Only in $root/.claude/"*)     rel="${line#"Only in $root/.claude/"}";     printf 'main-only %s\n'   "${rel/: //}" ;;
      Files*differ)                  rel="${line#"Files $root/.claude/"}";       printf 'changed %s\n'     "${rel%% and *}" ;;
    esac
  done
}

skill_of() { local p="${1#skills/}"; printf '%s' "${p%%/*}"; }

# A symlink under the worktree's .claude/ whose target does not resolve from
# there — the skill is unusable through the worktree path.
dangling() { [ -L "$worktree/.claude/$1" ] && [ ! -e "$worktree/.claude/$1" ]; }

if [ "$tool" = "Skill" ]; then
  skill=$(printf '%s' "$input" | jq -r '.tool_input.skill // ""')
  skill="${skill##*:}"
  [ -n "$skill" ] || exit 0
  [ -d "$root/.claude/skills/$skill" ] || exit 0

  if dangling "skills/$skill"; then
    branch=$(git -C "$worktree" rev-parse --abbrev-ref HEAD 2>/dev/null)
    emit "The \`$skill\` skill above was loaded from the main checkout ($root/.claude/skills/$skill), and that copy is the one to use: this session works in the worktree of branch \`$branch\`, where the skill is a symlink (\`$(readlink "$worktree/.claude/skills/$skill")\`) that does not resolve. Read its reference files under $root/.claude/skills/$skill, never via $worktree/.claude/skills/$skill. A session launched inside the worktree would not have this skill at all."
  fi

  changes=$(claude_diff "skills/$skill")
  [ -n "$changes" ] || exit 0

  branch=$(git -C "$worktree" rev-parse --abbrev-ref HEAD 2>/dev/null)
  skill_dir="$worktree/.claude/skills/$skill"
  msg="The \`$skill\` skill above was loaded from the main checkout ($root/.claude/skills/$skill), but this session works in the worktree of branch \`$branch\`, and that branch changed the skill:"
  msg="$msg
$(printf '%s\n' "$changes" | sed 's|^\([a-z-]*\) skills/[^/]*/|- \1: |')"
  msg="$msg

The branch's version supersedes what the Skill tool returned, and the skill's base directory for this session is \`$skill_dir\` — read its reference files from there."
  if [ -f "$skill_dir/SKILL.md" ]; then
    msg="$msg

--- $skill_dir/SKILL.md ---
$(cat "$skill_dir/SKILL.md")"
  else
    msg="$msg

The branch has no SKILL.md for it any more — the skill is removed on this branch, do not follow the version above."
  fi
  emit "$msg"
fi

# EnterWorktree
changed_skills=""; branch_only_skills=""; main_only_skills=""; other=""
for area in skills hooks agents settings.json; do
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    kind="${line%% *}"; rel="${line#* }"
    case "$rel" in
      skills/*)
        name=$(skill_of "$rel")
        case "$kind" in
          changed)     case " $changed_skills " in *" $name "*) ;; *) changed_skills="$changed_skills $name" ;; esac ;;
          branch-only) if [ "$rel" = "skills/$name" ]; then branch_only_skills="$branch_only_skills $name"; else changed_skills="$changed_skills $name"; fi ;;
          main-only)   if [ "$rel" = "skills/$name" ]; then main_only_skills="$main_only_skills $name"; else changed_skills="$changed_skills $name"; fi ;;
        esac ;;
      *) other="$other
- $kind: .claude/$rel" ;;
    esac
  done <<<"$(claude_diff "$area")"
done
changed_skills=$(printf '%s\n' $changed_skills | sort -u | tr '\n' ' ')

dangling_skills=""
for entry in "$worktree"/.claude/skills/*; do
  [ -e "$entry" ] || [ -L "$entry" ] || continue
  name="${entry##*/}"
  dangling "skills/$name" && dangling_skills="$dangling_skills $name"
done

[ -z "$changed_skills$branch_only_skills$main_only_skills$other$dangling_skills" ] && exit 0

branch=$(git -C "$worktree" rev-parse --abbrev-ref HEAD 2>/dev/null)
msg="This session loads skills, hooks, agents and settings from the main checkout ($root), not from the worktree it just entered, and branch \`$branch\` differs there:"
if [ -n "$dangling_skills" ]; then
  msg="$msg

Symlinked skills whose link does not resolve inside the worktree — the Skill tool still loads them from the main checkout, so load them normally and read their files there, never via the worktree path (a session launched inside the worktree would not have them at all):$(for s in $dangling_skills; do printf '\n- %s — link `%s`, files at %s/.claude/skills/%s' "$s" "$(readlink "$worktree/.claude/skills/$s")" "$root" "$s"; done)"
fi
if [ -n "$changed_skills" ]; then
  msg="$msg

Skills changed on the branch:$(for s in $changed_skills; do printf '\n- %s — when you load it, the branch version arrives with it; its base directory is %s/.claude/skills/%s' "$s" "$worktree" "$s"; done)"
fi
if [ -n "$branch_only_skills" ]; then
  msg="$msg

Skills that exist only on the branch — they are not in the skill listing, the Skill tool cannot load them; read the file instead when the work needs the skill:$(for s in $branch_only_skills; do printf '\n- %s — %s/.claude/skills/%s/SKILL.md' "$s" "$worktree" "$s"; done)"
fi
if [ -n "$main_only_skills" ]; then
  msg="$msg

Skills removed on the branch (still listed and loadable here, but the branch does not have them):$(for s in $main_only_skills; do printf '\n- %s' "$s"; done)"
fi
if [ -n "$other" ]; then
  msg="$msg

Hooks, agents or settings changed on the branch — the main checkout's copies are what runs in this session, the branch's changes to them are not active:$other"
fi
msg="$msg

If the branch is about these files themselves, a session launched inside the worktree (\`claude\` run from $worktree) loads all of them from the branch."
emit "$msg"
