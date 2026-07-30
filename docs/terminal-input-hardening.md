# Terminal and Text Input Hardening

Status: **Implemented**

This document is the contract for s7s terminal ownership, pasted input, and
user-perceived text boundaries. It records what the code does, why each decision
was taken, and what is deliberately out of reach.

## Decision summary

| Area | Decision |
| --- | --- |
| Terminal ownership | RAII `TerminalSession` (owns the ratatui terminal + a `TerminalModes` flag set) replaces the free `init_terminal` / `restore_terminal` pair |
| Restoration | Attempt every still-active cleanup step even when an earlier step fails; return the first error; keep failed steps flagged so `Drop` retries them |
| Panic | A panic hook restores the terminal *before* the message is printed; `Drop` alone would erase the message with the alternate screen |
| Testability | A `TerminalOps` trait isolates the crossterm calls, so ordering, flag bookkeeping, and failure recovery are unit tested with injected failures |
| Paste protocol | crossterm bracketed paste is enabled; one user paste is handled as one `Event::Paste(String)` |
| Paste routing | Routed to the field owning the caret in the current `UiMode` (exhaustive match); non-input modes ignore it |
| Single-line paste | Normalize CRLF/CR, line breaks and tabs become spaces, other control characters are discarded, capped at 4096 bytes on a grapheme boundary |
| Terminal command field | Multi-line paste keeps **only the first line** (joining shell lines with spaces changes what runs), and reports that it did |
| Unicode normalization | None on paste — the pasted bytes are preserved (a rename title must keep the exact bytes the agent CLI stored) |
| Cursor storage | Byte offset kept on a `char` boundary; movement and deletion step over whole extended grapheme clusters |
| Insertion cursor | Left-biased: the cursor lands at the end of inserted text and is never snapped forward over a following combining mark |
| Display width | `unicode-width`, measured **per grapheme cluster**; `unicode-segmentation` owns editing and string-splitting boundaries |
| Signals | `Drop` cannot recover from `abort`, `SIGKILL`, power loss, or unhandled terminating signals; `reset` stays the documented manual recovery |

## Why this was needed

### Partial terminal setup and teardown leaked state

The old `init_terminal` enabled raw mode and entered the alternate screen before
the terminal handle was constructed, so a later failure returned without undoing
the earlier steps. `restore_terminal` returned at the first `?`: if raw mode could
not be disabled, leaving the alternate screen and showing the cursor were never
attempted. Nothing owned the terminal, so an early return or an unwind panic had
no cleanup path at all.

### A paste could be interpreted as executable key input

Bracketed paste was not enabled and `runtime::dispatch_event` did not handle
`Event::Paste`, so terminals decoded pasted text as individual key events.

The sharpest case was the `!` Quick Command window: a pasted newline arrives as
`KeyCode::Enter`, which *runs* the command in the session folder. Pasted `:` and
`!` could also trigger the empty-input mode switches instead of being inserted.

### `char` boundaries do not match user-perceived characters

`TextInput`, the session keyword cursor, `truncate_w`, and `wrap_w` split strings
at Rust `char` boundaries. That protects UTF-8 validity but still splits a
combining sequence (`e` + U+0301), a ZWJ emoji, or an emoji plus a variation
selector — producing partial deletion, surprising cursor jumps, or a dangling
joiner at a truncation boundary.

Per-`char` measurement was also *wrong about width*: `unicode-width` reports the
real terminal width of a ZWJ sequence (2 columns) only when the whole cluster is
measured at once. Summing the parts gives 8 for a family emoji, so grapheme
measurement fixes column accounting as well as splitting.

## Scope

In scope: terminal setup/suspend/resume/cleanup; the agent, login, new-session and
shell-command handovers; the temporary raw-mode prompts used while the TUI is
suspended; every single-line editable field including the folder-filter query;
keyword search cursor movement; width-aware truncation and wrapping; and the unit,
routing, fault-injection and PTY checks for all of it.

Not in scope: mouse capture; multiline editor widgets; key-binding changes; moving
away from `unicode-width` for display width; an async runtime; and any promise of
recovery after `SIGKILL`, `process::abort`, `panic = "abort"`, power loss, or an
unhandled terminating OS signal.

## 1. Grapheme-safe text editing

`unicode-segmentation` is a direct dependency. It was already present in
`Cargo.lock` through ratatui's `unicode-truncate`, so declaring it adds no new
crate to the build — but s7s must declare every crate it imports.

`TextInput::cursor` stays a byte offset so slicing stays cheap.
`src/ui/components/input.rs` exposes `prev_grapheme_boundary` and
`next_grapheme_boundary`, which replace the old `prev_char_boundary` /
`next_char_boundary` pair.

Invariants:

1. `cursor <= value.len()`;
2. `value.is_char_boundary(cursor)` — every entry point clamps defensively;
3. left/right movement advances by one grapheme cluster;
4. Backspace/Delete removes one complete cluster when the cursor sits on a
   cluster boundary;
5. insertion is **left-biased**: the cursor lands immediately after the inserted
   bytes.

Point 5 is a deliberate departure from "snap the cursor to the next cluster
boundary". Snapping forward would step over a combining mark that follows the
insertion point and make that mark unreachable — the user could never delete it.
The cost is that the cursor may briefly sit inside a cluster (only reachable by
inserting *before* an existing mark); the boundary helpers accept such an offset
and resolve to the enclosing cluster edges, so no operation can split a cluster.

The Session search keyword keeps its string and cursor on `App` rather than in a
`TextInput`. It uses the same helpers and the same paste-insertion function
(`insert_paste_at`), so the two input implementations cannot disagree.

`ui::render::input_view` scrolls its horizontal viewport by grapheme cluster while
still computing the cursor column with `UnicodeWidthStr::width`.

## 2. Grapheme-safe truncation and wrapping

`truncate_w_with_ellipsis` and `wrap_w` iterate `graphemes(true)` and measure each
cluster with `UnicodeWidthStr::width`.

Rules:

- the result never exceeds the requested terminal-cell width;
- an ellipsis is reserved before the last visible cluster is added;
- no output boundary exposes part of a cluster;
- tabs still expand to four-column tab stops;
- truncation of a cluster wider than the available width emits a width-safe
  ellipsis;
- **wrapping** of such a cluster puts it on its own line instead — wrapping must
  never lose content, so the "ellipsis instead of overflow" rule applies to
  truncation only.

`chars()` is unchanged in domain logic where the intended unit really is a Unicode
scalar or an approximate character count; this change is limited to UI editing and
UI string boundaries.

## 3. Bracketed paste

`EnableBracketedPaste` is part of TUI entry and `DisableBracketedPaste` part of
every suspension and of final cleanup, so the protocol never leaks into a child
agent CLI or the parent shell. `crossterm`'s `bracketed-paste` feature is on by
default, so no Cargo feature change was required.

`runtime::dispatch_event` forwards `Event::Paste(text)` to `App::on_paste`. The
paste stays one event and one edit; it is never expanded into synthetic
`KeyCode::Char`/`KeyCode::Enter` events. Execution and submission still require a
physical Enter key press.

### Sanitization

`src/ui/components/input.rs` owns it, in this order:

1. `\r\n` and lone `\r` normalize to `\n`;
2. line breaks and tabs become single spaces (`insert_paste` / `insert_paste_at`),
   or the text is cut at the first line break (`insert_paste_first_line`);
3. remaining control characters are discarded (not replaced);
4. all other Unicode text is preserved byte-for-byte (no NFC/NFD normalization);
5. the payload is capped at `MAX_PASTE_BYTES` (4096) on a grapheme boundary;
6. the result is inserted at the cursor as one edit.

The cap exists because an unbounded paste would be re-measured for display width
on every frame and, for rename, written into agent session metadata.

`PasteOutcome` reports what had to be dropped, and `App::note_paste_outcome` turns
that into a status message — silent when the whole payload landed.

### Paste routing matrix

| `UiMode` | Target | Post-edit action | Line breaks |
| --- | --- | --- | --- |
| `Keyword` | session keyword at `keyword_cursor` | `recompute()` | spaces |
| `Rename` | rename `TextInput` (Input focus only) | none | spaces |
| `ProfileForm` | focused Name or Path input only | clear `form.error` | spaces |
| `NewSession` | folder input, only while that row is focused | `on_input_edited()` | spaces |
| `QuickCommand` (palette) | the shared `QuickState.input` | `quick_recompute()` | spaces |
| `QuickCommand` (terminal) | the same input field | `term_after_edit()` | **first line only** |
| `FolderModal` | `folder_query` (appended: the query has no cursor) | `refresh_folder_modal_labels()` | spaces |
| `Table`, `AgentModal`, `DeleteConfirm`, `ProfileDeleteConfirm`, `ProfileDirConfirm`, `ProjectDirConfirm`, `ThemeSelect`, `Help`, `Message` | none | — | ignored |

Notes:

- Palette and terminal mode **share one `TextInput`** (`QuickState.input`); only
  the post-edit hook and the line-break policy differ by `state.mode`.
- The terminal-command row is a safety boundary. A pasted newline must not become
  Enter (which would run the command) and must not become a space either: gluing
  shell lines together changes semantics — a trailing `#` comment would swallow
  the following line. Only the first line is kept, and the user is told.
- `FolderModal` is included even though its query is a bare `String` with no
  cursor, because it is a real editable field; pasted text is appended.

## 4. RAII terminal ownership

`runtime.rs` defines three layers:

- `TerminalOps` — the raw crossterm calls (`enable`/`disable` per mode, show
  cursor, keyboard-enhancement capability query). `CrosstermOps` is the real
  implementation; tests use an in-memory mock.
- `TerminalModes<O: TerminalOps>` — the policy: ordering, per-mode flags, and
  failure recovery. This is the unit-tested core.
- `TerminalSession` — owns the ratatui `Tui` plus `TerminalModes<CrosstermOps>`,
  and restores the terminal in `Drop`.

### Entry

`enter()` enables raw mode, the alternate screen, and bracketed paste in that
order, recording each success immediately. Raw mode must come first: the
keyboard-enhancement capability query needs it. Keyboard enhancement is optional —
an unsupported or failing terminal keeps legacy input, where the `:` palette
remains the functional fallback for `ctrl+shift+n`.

If a required step fails, the modes already enabled are turned back off before the
error propagates, so a half-configured terminal is never handed back to the shell.
`enter()` skips modes that are already on, which makes it double as `resume()`.

The old `KEYBOARD_ENHANCED` global was replaced by a session flag. A separate
`ACTIVE_MODES` bitmask remains as the single source of truth for the *real*
terminal: the panic hook needs it (it does not own the session), and it makes a
redundant enable/disable a no-op — which is what stops the unwinding
`TerminalSession::drop` from popping a second keyboard-enhancement level that
would belong to whatever process launched s7s.

### Cleanup

Order: keyboard enhancement flags → bracketed paste → alternate screen → show
cursor → raw mode.

Cursor visibility is restored *after* leaving the alternate screen (some terminals
track it per screen buffer, so showing it first can leave the main screen with a
hidden cursor) and unconditionally, because ratatui hides the cursor while drawing
rather than through a mode flag.

Every applicable step is attempted even when an earlier one fails, and the first
error is returned, named with the mode that failed. Only successful steps clear
their flag, so a retry — an explicit call, or `Drop` — redoes exactly what is
still active.

The normal-exit path calls `suspend()` explicitly so its failure is reported: the
event-loop error is preserved and the cleanup error is attached as context, never
the other way round.

### Handovers

`run_loop` and every handover take `&mut TerminalSession`:

```text
session.suspend()
  -> print handover screen
  -> run child synchronously
  -> session.resume()      (re-enable modes, then clear)
  -> refresh data
  -> drain pending input
  -> begin quit grace
```

The same session is reused instead of replacing the terminal handle with a new
one. The temporary raw-mode prompts in `pause_before_return` and `offer_vim_retry`
use a smaller `RawModeGuard`, so an early return or panic inside a prompt cannot
leave raw mode enabled while the main session is suspended.

### Panic

`install_panic_hook` runs before the TUI takes the screen. The hook rebuilds the
mode set from `ACTIVE_MODES`, restores the terminal, and only then calls the
previous hook. This ordering is the whole point: unwinding prints the panic
message *first* and runs destructors afterwards, so a `Drop`-only restore would
print the message onto the alternate screen and then erase it.

### Termination limits

`Drop` runs on ordinary returns, `?` propagation, and unwind panics. It does not
run after `SIGKILL`, `process::abort`, `panic = "abort"`, power loss, or an
unhandled `SIGTERM`/`SIGHUP`. Signal-to-graceful-shutdown handling is a separate
change; documentation and release notes must not claim RAII covers these paths.
`reset` remains the documented manual terminal recovery.

## 5. Verification

`scripts/check.sh` covers everything automated below.

- **Text input** (`ui/components/input.rs` tests): grapheme movement and deletion
  over `e\u{301}` and a ZWJ family emoji; combining-mark insertion keeps the
  cursor after typed text; insertion before an existing mark does not skip it;
  paste sanitization (CRLF, CR, LF, tab, control characters); cursor-position
  insertion; first-line policy; the byte cap landing on a cluster boundary.
- **Text helpers** (`ui/components/text.rs` tests): truncation keeps combining
  sequences whole and never emits a dangling joiner; ZWJ/VS16 clusters measure as
  2 columns; ellipsis width is still reserved; wrapping preserves clusters; tab
  expansion unchanged; an over-wide cluster gets its own line without overflowing
  other lines.
- **Paste routing** (`ui/paste.rs` tests): every non-input mode ignores a paste;
  each field receives it exactly once with its post-edit hook; mode-switch
  characters insert literally; pasted newlines submit nothing; terminal mode keeps
  the first line, re-syncs the history filter, and creates no `terminal_request`
  until a real Enter arrives.
- **Terminal lifecycle** (`runtime::terminal_lifecycle_tests`, mock ops with
  injected failures): entry order (raw first); unsupported/failing keyboard
  enhancement is non-fatal; partial setup restores every mode already enabled;
  cleanup order; one cleanup failure does not skip later steps; the first error is
  returned; flags clear only on success; a retry redoes only what stayed active;
  the panic-hook mask restore touches only recorded modes; suspend/resume are
  balanced and idempotent.

Manual/PTY checks (not part of `check.sh`) are listed in
[testing.md](./testing.md#terminal-lifecycle-and-paste-checks), including the
debug-only `S7S_PANIC_PROBE` fault injection used to verify that a panic restores
the terminal *and* leaves its message readable.

## 6. Files owning this behavior

| File | Role |
| --- | --- |
| `Cargo.toml` | direct `unicode-segmentation` dependency |
| `src/ui/components/input.rs` | grapheme helpers, `TextInput`, paste sanitization and insertion, cap |
| `src/ui/components/text.rs` | grapheme-safe truncation and wrapping |
| `src/ui/paste.rs` | `App::on_paste` routing, `note_paste_outcome` |
| `src/ui/mod.rs` | helper re-exports (`insert_paste_at`, boundary helpers, `PasteOutcome`) |
| `src/ui/render.rs` | grapheme-aware `input_view` viewport |
| `src/ui/session/input.rs` | grapheme keyword editing + `paste_into_keyword` |
| `src/ui/overlays/confirm.rs` | `paste_into_rename` |
| `src/ui/overlays/filters.rs` | `paste_into_folder_query` |
| `src/ui/profile/input.rs` | `paste_into_profile_form` |
| `src/ui/new_session/input.rs` | `paste_into_new_session` |
| `src/ui/quick/input.rs` | `paste_into_quick` (palette / terminal policies) |
| `src/runtime.rs` | `TerminalOps`, `TerminalModes`, `TerminalSession`, `RawModeGuard`, panic hook, `Event::Paste` dispatch, handovers |

## 7. Constraints on future changes

- A new `UiMode` must extend the `App::on_paste` match explicitly — the match is
  exhaustive on purpose, so "insert somewhere by default" cannot happen silently.
- A new editable field must use `TextInput` or `insert_paste_at`; hand-rolled
  `chars()` cursor arithmetic reintroduces the split-cluster bug. The line-break
  policy is not a caller-visible argument: a field opts into first-line-only
  behavior by calling `insert_paste_first_line`, so the safe default cannot be
  lost in a parameter.
- Terminals without bracketed paste cannot be detected (crossterm exposes no
  query). There, pasted text still arrives as key events and the `!` hazard
  remains; the guarantee below is scoped accordingly.
- "One paste, one event" is also not fully in s7s's control: a terminal or
  multiplexer may chunk a large paste. The invariant s7s enforces is that no paste
  is interpreted as key input, not that chunking never happens.

## 8. Guarantees

- `scripts/check.sh` passes.
- Every editable field operates on grapheme boundaries.
- `truncate_w` and `wrap_w` never split a cluster, and measure ZWJ/VS16 sequences
  at their real terminal width.
- In a bracketed-paste-capable terminal, one paste produces one input mutation and
  pasted line breaks cannot submit a form or execute a terminal command.
- Normal exit, recoverable errors, an unwind panic, and every handover attempt all
  applicable cleanup steps; a panic's message stays readable.
- Keyboard enhancement and bracketed paste do not leak into child processes or the
  parent shell (verified against the PTY's termios and escape-sequence trace).
- Unsupported terminals keep the existing Quick Command fallback.
- No mouse capture is introduced.
