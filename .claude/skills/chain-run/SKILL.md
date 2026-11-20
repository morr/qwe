---
name: chain-run
description: Use when a session is a link of qwe's continuous self-spawning chain of improvement runs — the prompt says «непрерывный прогон», «звено №N», «обзор №N», «прогон №N» — or when asked to start, resume or restart that chain («продолжи цепочку», «запусти следующий прогон», «цепочка умерла»). Carries the link protocol (read NOTES, work in a worktree, check.sh, ff, NOTES section, spawn + liveness-check the next link) and the chain's standing traps — cwd drift, App Nap, non-alternating A/B.
---

# The continuous run chain — link protocol

qwe runs an endless self-spawning chain of improvement sessions. Each link is one
ZCode session; its **only correct end is the next link alive**. «Проблем больше нет»
is not a stop condition — the next link looks deeper or under another angle.

Division of labor: **this skill is the mechanics, `NOTES.md` is the content**. The
convention and the owner's directives live in `NOTES.md` (main checkout), section
«След сессии — бесконечная цепочка обзорных прогонов»; the run history is the link
sections above it, newest first; the permanent sections («Известные измерения»,
«Уроки живых замеров», «Рабочее меню оптимизаций», «Углы, взятые цепочкой») sit at
the very top and every link updates them. When a steer in NOTES is fresher than
this file, the steer wins.

## The link loop

1. **Gated skills before the first gated action**: `git`, `bevy`, the domain skill of
   the area the change will touch; `live-app` before the first `cargo run` / `brp`
   call (the hook denies otherwise). Reload after a compaction.
2. **Read the run state**: the permanent sections at the top of `NOTES.md`, the latest
   link sections, and «След сессии» (directives — the newest steer decides the form:
   currently code optimizations unless a later steer says otherwise).
   `docs/deepening-plan.md` is uncommitted by design — read it, never commit it.
   Record the current `git rev-parse --short HEAD` as «master на старте».
3. **Clean up the parent link** (it cannot clean up after itself — its bash and any
   lingering builds live inside its worktree): the parent's NOTES section names its
   branch («Найдено и влито (ветка perf19, …)»). Remove that worktree and branch:
   `git worktree remove .worktrees/<name>` (skip only if the parent's section
   explicitly says the worktree is kept — an A/B pair or a measurement base) and
   `git branch -d <name>` (plain `-d`: it refuses on unmerged branches, which is the
   protection for deliberately-kept experiment branches like perf12/perf15).
4. **Pick the work per the current form**: optimizations take ready targets from the
   run state, or a fresh cut of the largest remaining wall; the review form takes a
   fresh angle (see «Углы, взятые цепочкой» — never repeat one). One session = one
   menu item or a few small optimizations, not a marathon. A non-perf problem that is
   safe and small gets fixed; risky/large work is recorded in `docs/deepening-plan.md`
   as deferred with a trigger, not done.
5. **Work in a worktree** `.worktrees/<name>` inside the project dir. Seed the build
   dir with `tools/seed-worktree-target.sh .worktrees/<name>/target` (the argument is
   the build dir, not the worktree root); `rsync -a` `assets/osm` preserving mtime.
   Right after your **first edit**, run `git status --short` — it catches edits that
   landed in the main checkout (bitten in №5/№14).
6. **cwd discipline** (bitten in №3/№5/№7/№14/№16/№17/№18/№19): the Bash cwd is the
   main checkout by default and persists in **both** directions; `cd` inside a
   background task's command does not persist at all. Switch cwd with a separate
   short `cd <worktree> && pwd` call before background builds; write edit paths after
   the cd; the app launch line is `cd <worktree> && pwd && QWE_PERF_NOAPPNAP=1
   caffeinate -i env BRP_PORT=15703 cargo run`.
7. **Verify**: `tools/check.sh <changed .rs files>` as one background task **by
   absolute path** from the worktree (it takes no `.md` / `Cargo.toml` — rustfmt
   E0758; do not edit files while it runs). Per-CLAUDE.md perf verdict on every
   change, out loud; hot-path changes get an alternating live A/B (base/swap/base —
   the machine drifts ±70 ms between sessions, consecutive runs lie). A merged
   optimization gets its perf-journal entry: `docs/perf-journal/NN-<slug>.md` + the
   README line + the `qwe-perf-journal` artifact (`get_artifact` → edit →
   `update_artifact` with `expectedRev`). Remove the TEMP probe instrumentation
   before committing (`grep TEMP` over the diff).
8. **Merge**: small commits per the `git` skill; ff master from the main checkout
   (`git -C /Users/morr/develop/bevy/qwe merge --ff-only <branch>`); if master moved,
   rebase and re-check the build. After the ff — a cheap `cargo check --lib` in the
   main checkout (the review-4 lesson: a green worktree check.sh once left master
   broken). Doc twins (CONTEXT.md + the owning skill) land in the same change as the
   code.
9. **Write your NOTES.md section** on top (main checkout), commit it: what was
   scanned/found/merged (branch, commits, check status), negatives with reasons, the
   perf verdict, and «Остатки и уроки» for the next links. The section must name your
   branch, and must explicitly say «worktree <name> сохранён …» when you keep the
   worktree for the next links (an A/B pair or a measurement base) — the child link
   deletes every parent worktree that is not marked kept. Update the permanent
   sections if you produced new numbers or took a new angle.
10. **Spawn the next link** per the contract below, and only end the session after
    the child is confirmed alive. While measuring live, keep subagents to one at a
    time, foreground; no parallel compiles beside the app.

## The spawn contract

The chain used to reproduce a 16K-char prompt by hand — every link rewrote it, and
it drifted. The prompt is now short; the child loads this skill and reads NOTES for
content. NOTES.md is the prompt's backup copy: the permanent sections carry
everything the old prompt carried.

```
spawn_session(
  prompt: "Ты — звено №{N+1} непрерывного прогона qwe. Загрузи скилл chain-run
           и следуй его протоколу звена. master на старте: {sha}. Форма —
           по директивам из NOTES «След сессии» (свежайший steer решает).",
  directory: "/Users/morr/develop/bevy/qwe",
  model: "account:zai-start-plan/GLM-5.3-Flash",
  title: "qwe обзор №{N+1} — непрерывный прогон",
  task: false)
```

- `task: false` always — the chain lives in the project's session list, not Tasks.
- Omit `mode` (the child inherits; the chain runs yolo — never an asking mode for
  unattended work) and keep `thought_level: max`.
- **Liveness check — mandatory** (the №20 death: the child's first model call failed
  at 03:24, the spawner had ended 3 s earlier, and the chain lay dead for 12 h).
  After the spawn returns the child's id: run `sleep 90` as a background Bash task;
  when it exits, `get_session(<child>)`. The child is alive when its parts grow past
  the prompt (skill loads, reads). `last_outcome: error` or no parts → read the
  failure first: `search_log(session_id: <child>, query: "model.request.failed")`
  and check `statusMessage`.
  - `exceed quota limit` (retryable false) — the free-plan tokens are exhausted:
    **this is the chain's expected stop, do not respawn.** Add one line to the top of
    NOTES.md («цепочка остановлена: квота бесплатного плана, <дата>»), commit, end
    the session. The owner restarts the chain when the quota resets.
  - anything else (network, transient) → fix the cause and spawn again — same
    number, fresh session. Repeat until alive.
- A child that 429s its first turns (concurrency with your own session) is alive — the
  scheduler self-heals with «continue» injections; do not respawn it.

## Compaction — the chain cleans up after itself

Without compaction the run's artifacts grow without bound: NOTES.md gained
~1900 lines in four days of the chain. Every link checks two things (the
worktree/branch half is already step 3 of the loop):

1. **NOTES.md**: when more than five link sections sit below the permanent
   sections, cut the oldest ones out — whole sections, from their
   `# NOTES — <link>` header to the next one — and append them to
   `docs/run-history/<yyyy-mm>-links.md` (one header line with the cut date),
   committing both files together («archive old run sections of NOTES to
   docs/run-history»). The permanent sections and the convention section
   («След сессии — бесконечная цепочка…») never leave NOTES.md; if the
   convention has drifted into an old section, lift it back up first.
2. **Session tracker**: the project session list fills with dead links. Archive
   the sessions of links older than the last five:
   `list_sessions(query: "непрерывный прогон")` → older ids →
   `SetSessionArchived(sessionId, archived: true)` each (reversible, does not
   interrupt). The run's memory lives in NOTES and commits, not in the session
   list.

## Restarting a dead chain

Any session asked to resume the chain: find the last link with
`list_sessions(query: "непрерывный прогон")` (newest first) and the top section of
`NOTES.md` (its number N). If the last link ended in error or the chain stopped
overnight — spawn link №N+1 per the contract, with the current HEAD as
«master на старте». Two expected, non-emergency ways the chain lies dead: the
free-plan quota ran out (`exceed quota limit` — restart when the quota resets) and
the machine slept overnight. The first link after a restart also sweeps the leftover
worktrees/branches of the dead tail (step 3 rules: kept ones are marked in their
NOTES sections, unmerged branches survive `git branch -d` by design).

The overnight sleep death (№6 lost 18 h: the app closed at 00:31 and automation
only resumed at 18:30 when it was opened again) cannot be prevented from inside a
session — the owner keeps the machine awake (caffeinate/Amphetamine) for an
unattended run, or restarts the chain in the morning, which is this section.
