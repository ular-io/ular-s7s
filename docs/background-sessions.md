# Claude Background Sessions

> Read when: changing the Claude live-session markers (`Ⓑ` background, `Ⓞ` open),
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
so opening a session whose worker is alive fails with "Agent exited abnormally
(exit code: 1)". Opening it correctly (`claude attach <jobId>` when a live
worker holds it) is not implemented yet.

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
| `interactive` | `Ⓞ` | `Open`: open in another Claude Code terminal |

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
bounded by the next due time). A sweep replaces each profile's map; a failed
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

- Status can be up to 30 s old; a session backgrounded since the last sweep
  still fails to open.
- The forked original is still listed beside its copy (no folding by
  `continued-in` yet). The marker identifies the copy.
- Whether a terminal is attached to a background session is not observable
  (the daemon keeps attachers in memory only), so `Ⓑ` cannot tell a session
  someone is watching from an unattended one.

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
   confirm `Ⓞ` appears and clears after that terminal exits.
4. Re-check the resume table above with and without
   `--dangerously-skip-permissions`, then `claude rm <id>`.
