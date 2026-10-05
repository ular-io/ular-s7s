# Live Sessions: Claude Background and Open Elsewhere

> Read when: changing the live-session markers (`Ⓑ` Claude background, `Ⓞ`
> open in another terminal, for Claude and agy),
> `src/agent_status.rs`, `src/ui/agent_status.rs`, or anything that depends on
> how Claude Code's daemon holds a session.

Observed on Claude Code 2.1.288. Every behavior below is external and must be
re-verified on a CLI upgrade.

## What a background session is

Claude Code can run a session inside its daemon (`claude daemon run`) instead of
the terminal that started it. The daemon keeps a worker process per session and
a PTY host the terminal attaches to; closing the terminal leaves the work
running.

A session becomes background by:

- `←` on an **empty** prompt (no confirmation; while a turn is running it waits
  for the current tool, a second `←` skips waiting for subagents). If the input
  was cleared within the last 2 s, the first press only arms and a second one
  within 3 s fires.
- `/background` (`/bg`), `claude --bg "<prompt>"`, a task dispatched from the
  agent view (`claude agents`), and the exit dialog's "Move to background and
  exit".

Backgrounding a running session **forks it under a new session id**. The
original transcript keeps everything up to the fork plus a
`{"type":"continued-in","continuedInSessionId":"<new id>"}` record; the new
transcript repeats the conversation, marks every line `sessionKind: "bg"`, and
gains an `agent-name` record (so s7s shows the auto name there even when the
original falls back to its first prompt). Each further `←` in a foreground
session forks again (`name (2)`, `name (3)`, …). Attaching to an already
background session and leaving with `←` only detaches; it never forks.

## Lifecycle

- Per config dir (`CLAUDE_CONFIG_DIR` or `~/.claude`):
  `sessions/<pid>.json` lists live processes (`kind: interactive|bg`,
  `sessionId`, `jobId`), and `jobs/<short id>/state.json` keeps one record per
  background job (`state`, `respawnFlags`, …) even after its worker exits.
- A finished turn leaves the worker alive (`state: done`). The daemon checks
  every 60 s and retires a worker with no attachment, no in-flight work, and no
  input or update for 1 h (1 min under memory pressure). The job record stays;
  the agent view can reopen it, respawning with `respawnFlags`. The daemon
  exits 5 s after its last worker and client are gone.
- `claude stop <id>` ends the worker and keeps the conversation; `claude rm <id>`
  deletes the job and its worktree.

## Resume behavior

Resume checks only **live** non-interactive holders of the session id
(`listAllLiveSessions`); job records alone do not block it.

| Session | `claude --resume <id>` | `claude --resume <id> --dangerously-skip-permissions` |
| --- | --- | --- |
| Live background worker (`pid` present) | Attaches to the running worker ("Opening the background session …") | **Refused, exit 1**: "That session is running in the background (…). Run `claude attach …` … or `claude stop …` first …". The message goes to the TTY, not stderr |
| Worker retired by the daemon | — | Opens as a normal session |
| After `claude stop` | — | Opens as a normal session |

The refusal comes from `cliSessionConfigCarried`: a flag that configures the
session (the permission mode here) cannot be applied to an already running
worker, so Claude refuses instead of attaching. Auto-attach also needs a TTY,
the agent view enabled, and the server flag `tengu_resume_open_live_bg`.

Permission mode is not restored from the transcript: a normal resume uses the
command's flags or the settings default (`permissions.defaultMode`). Attaching
keeps the worker's own launch mode (`respawnFlags`).

s7s's default `resume_claude` template carries `--dangerously-skip-permissions`,
so resuming a session whose worker is alive would fail with "Agent exited
abnormally (exit code: 1)"; s7s therefore attaches instead (below).

## Attaching

Several terminals can attach to one worker at once (the daemon keeps a set of
attachers). There is still one writer, so the transcript is safe, but every
attached terminal types into the same prompt and the worker's screen is resized
to the newest attacher. Who is attached is held only in daemon memory: neither
`agents --json`, the registry, `roster.json`, nor process command lines and
environments reveal which session an agent-view terminal is showing (it can
switch after launch), so s7s cannot tell a watched session from an unattended
one.

`claude attach <jobId>` returns exit 0 when the user leaves with `ctrl+z`, or
with `←` (agent view) and then `ctrl+c` twice; the worker keeps running either
way (verified on 2.1.288). That makes it a normal blocking handover for s7s.

## Actions on live sessions

Right before Enter (resume), delete, or rename, s7s queries the session's
profile again (`App::live_entry_now`, refreshing that profile's markers), so a
30 s old sweep cannot let a live session through. A failed query falls back to
the last sweep.

| Holder | Enter | Delete / Rename |
| --- | --- | --- |
| Background worker (`Ⓑ`) | Attach/Cancel dialog (`ui/overlays/attach.rs`), **Cancel focused**, warning that an attached terminal would share screen and input; Attach runs `resume::run_attach` in a handover | Blocked with a message naming `claude stop <jobId>` |
| Another terminal (`Ⓞ`) | Blocked with a message | Blocked with a message |
| None | Resume as before | As before |

`s7s session delete` and `s7s session rename` refuse the same way
(`agent_status::live_holder`, exit 1 with an `error:`/`hint:` pair), including
a `delete` dry run. A retired worker no longer blocks anything, so a session
can be deleted once the daemon retires it (about 1 hour idle).

## Status markers

`src/agent_status.rs` runs `claude agents --json` per Claude profile (profile
env via `Profile::env_var`, Claude session variables stripped by
`resume::sanitize_agent_env`, 10 s timeout). The command reads the on-disk
registry: it neither starts a stopped daemon nor writes files (verified
against a profile whose daemon was not running), and takes ~0.3 s.

Only entries **with a `pid`** are kept; without a live worker a background
session resumes normally, and a profile without a daemon shows no background
marker.

| `kind` / `state` | Marker | Prompt legend (`LiveStatus`) |
| --- | --- | --- |
| background `working` (and unknown values) | `Ⓑ` | `Working`: a turn is running |
| background `blocked` | `Ⓑ` | `NeedsInput`: waiting on a permission prompt or user input |
| background `done` | `Ⓑ` | `Done`: turn finished; worker still alive until retired |
| `interactive` | `Ⓞ` | `Open`: open in another terminal |

The list shows one marker per way of opening: every background state opens the
same way, so they share `Ⓑ` and only the Prompt legend row tells them apart.

`Ⓞ` matters because Claude Code does not refuse a resume of a session another
terminal holds (the resume check looks only at non-interactive holders), so two
processes would append to the same transcript. `agents --json` already omits
the interactive process left behind by `←` (its registry entry carries
`parkedJobId`), so the forked original is not marked. s7s's own usage/model
probe sessions appear as interactive entries but are not in the session list.
When one session has both kinds of holder, the background marker wins. The
registry follows a session switch inside one process: `/clear` and in-session
`/resume` rewrite its `sessionId` immediately, and `/exit` removes the entry
(verified on 2.1.288). Only CLI sessions (`entrypoint: cli`) have been
observed; IDE and desktop sessions are unverified.

`src/ui/agent_status.rs` schedules sweeps: on the first loop pass after launch,
after every synchronous rescan (`refresh_sessions`: handover returns and
mutations), with `ctrl+u`, and every 30 s while idle (the event-loop wait is
bounded by the next due time). A sweep queries each Claude and agy profile on
its own thread (`agent_status::spawn_fetch`) and applies each profile's result
as it arrives, so one slow `claude agents --json` does not delay the markers of
other profiles. A sweep replaces each profile's map; a failed
query clears it, so a stale marker never outlives its source. Matching is by
`(profile_id, lowercase session id)` for Claude sessions only.

The marker follows the bookmark marker in the title prefix (`♥ Ⓑ  title`, see
[bookmarks.md](./bookmarks.md)) in the Session table, Prompt/Detail metadata,
and Context Source metadata. It is display-only and bold, like `♥`.
The Prompt pane's Session block adds one `- <marker> : <meaning>` row under
Name for each marker the session carries (`♥` first), from
`TitleMarks::legend` / `LiveStatus::description`. Detail and Context Source
blocks omit these rows, and copied session info never includes them.

Known gaps:

- Markers can be up to 30 s old. Actions re-query, so this only affects what
  the list shows.
- The forked original is still listed beside its copy (no folding by
  `continued-in` yet). The marker identifies the copy.
- Whether a terminal is attached to a background session is not observable
  (the daemon keeps attachers in memory only), so `Ⓑ` cannot tell a session
  someone is watching from an unattended one.

## Antigravity and Codex

Observed on agy 1.2.16 and Codex 0.160.0.

**Antigravity (`Ⓞ` only).** A running agy holds an exclusive `flock` on
`<profile root>/presence/<conversation id>.lock` (the conversation id is the
s7s session id) and releases it on exit; the file stays, so about 400 stale
files exist on a long-used machine. `agent_status::query_agy` reports the held
files. On macOS `F_GETLK` reports flock locks without acquiring anything, so
the check cannot collide with agy taking the lock; elsewhere it probes with a
non-blocking shared `flock` and releases it at once. No background or daemon
holder was observed, so agy sessions only ever show `Ⓞ`, and Enter, delete,
and rename block exactly as for a Claude `Ⓞ` session. Whether agy itself
refuses a conversation that is already open is unverified.

**Codex (not marked).** The terminal TUI talks to a shared app-server daemon,
which is the only writer and holds `thread-writer-locks/<thread id>.lock` plus
the rollout for every *loaded* thread. A thread stays loaded after its terminal
exits (still held 30 s later; subagent threads stayed loaded for hours), the
app-server protocol exposes thread status (`notLoaded`/`idle`/`active`/
`systemError`) but no attached clients, `tui-thread-reference-capabilities/`
only records threads ever opened, and a TUI's command line carries an id only
for `codex resume <id>`. Nothing tells which terminal shows which thread, so
Codex sessions get no marker and no block. With one daemon writing, two
terminals on one thread probably do not corrupt it (unverified).

## Verification

On a CLI upgrade, or when changing these modules:

1. Create a disposable background session from a scratch folder:
   `claude --bg --model claude-haiku-4-5-20251001 "Reply with exactly: OK"`.
2. `claude agents --json --all` — confirm `kind`, `state`, `status`, `pid`,
   `sessionId` still exist and that `pid` disappears after `claude stop <id>`.
3. In a release s7s, wait ≤30 s (or `ctrl+u`): the session shows `Ⓑ` and its
   Prompt legend reads `done` once its turn finishes; after `claude stop <id>`
   and the next sweep it shows nothing.
   Open a disposable session in another terminal (`claude --resume <id>`) and
   confirm `Ⓞ` appears and clears after that terminal exits. Repeat with a
   disposable agy conversation (`agy` in a trusted folder): `Ⓞ` while it runs,
   gone after it exits.
4. Re-check the resume table above with and without
   `--dangerously-skip-permissions`, then `claude rm <id>`.
