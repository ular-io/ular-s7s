# Terminal and Text Input Hardening

> Status: Current
> Read when: Changing terminal setup/cleanup, handovers, panic behavior, paste
> handling, editable text, cursor movement, wrapping, or truncation.
> Entry points: `src/runtime.rs`, `src/ui/paste.rs`,
> `src/ui/components/{input,text}.rs`

## Non-negotiable invariants

- `TerminalSession` owns every enabled terminal mode and restores all active
  modes on normal exit, error, unwind panic, and agent handover.
- Cleanup attempts every active step even if an earlier step fails, returns the
  first error, and leaves failed flags active so `Drop` can retry them.
- A panic hook restores the terminal before printing the panic message.
- Pasted text is data. It must never submit a form, switch modes, or execute a
  terminal command.
- Editable text, cursor movement, deletion, truncation, and wrapping operate on
  extended grapheme clusters, not Rust `char` boundaries.
- Child agent processes must not inherit s7s keyboard-enhancement or
  bracketed-paste modes.

## Text boundaries

`TextInput` stores the cursor as a byte offset on a UTF-8 character boundary,
but movement and deletion step over complete grapheme clusters using
`unicode-segmentation`.

- Insertion is left-biased: after inserting text, the cursor stays at the end of
  the inserted bytes and is never snapped forward over a combining mark already
  present to its right.
- Backspace/Delete removes one complete cluster.
- Home/End move to byte boundaries 0/`len`.
- `truncate_w`, `wrap_w`, and input viewport calculations measure complete
  clusters with `unicode-width`.
- Never slice display text by byte count or `char` count.
- An over-wide cluster gets its own line; do not split it to satisfy width.

## Whole-value selection

`TextInput::selected` marks a value the app filled in rather than the user typed
(`select_all`). It exists because the New Session folder prefill is a long
absolute path, and erasing one grapheme at a time to type a different folder is
the slowest possible way to reach the common case.

- Typing, pasting, `Backspace`, and `Delete` replace or clear the whole value.
  A deletion consumes the selection and stops there — it must not also delete a
  cluster.
- `←`/`Home` collapse to the start, `→`/`End` to the end, keeping the value. The
  edit flow therefore costs one key instead of being blocked.
- Only `TextInput::selected` arms the state; `TextInput::new` never does, so
  fields that add it opt in deliberately.
- Every entry point that mutates the value has to consume the selection first.
  Adding an editing method without that check silently appends to a value the
  user believes is about to be replaced.
- The state must be visible before the first key: the renderer paints a selected
  value with the selection colors ([ui-style-guide.md](./ui-style-guide.md)).

## Paste contract

`runtime` enables bracketed paste and forwards each `Event::Paste(String)` once
to `App::on_paste`. The exhaustive `UiMode` match routes it only to the field
that owns the caret; non-input modes ignore it.

### Single-line fields

Use the shared sanitization/insertion helpers:

1. normalize CRLF and CR to LF;
2. replace line breaks and tabs with spaces;
3. discard remaining control characters;
4. preserve Unicode bytes without normalization;
5. cap the result at 4096 bytes on a grapheme boundary.

The no-normalization rule is required for rename titles: the stored bytes must
match the external CLI.

### Terminal-command field

Use first-line-only insertion. Joining pasted shell lines with spaces can change
the command that runs. If later lines are dropped, report that fact; a paste
still must not create `terminal_request` until a separate Enter keypress.

### Routing ownership

| Field | Owner |
| --- | --- |
| Session keyword | `ui/session/input.rs` |
| Rename | `ui/overlays/confirm.rs` |
| Folder filter query | `ui/overlays/filters.rs` |
| Profile form | `ui/profile/input.rs` |
| New Session fields | `ui/new_session/input.rs` |
| Quick Command / terminal command | `ui/quick/input.rs` |

A new `UiMode` must extend `App::on_paste` explicitly. A new editable field must
use `TextInput` or the shared paste helpers.

## Terminal lifecycle

### Entry

`TerminalSession` applies modes in an order that permits partial rollback:

```text
raw mode
-> alternate screen
-> bracketed paste
-> optional keyboard enhancement
-> hide cursor / construct terminal
```

Keyboard enhancement is optional and non-fatal when unsupported. The active-mode
flags are the source of truth for what cleanup may attempt.

### Cleanup

Restore in the implementation-defined safe order in `runtime.rs`, attempting
every recorded mode. A successful step clears its flag; a failed step stays
recorded for retry. Cleanup must be idempotent.

Do not replace this stateful ownership with a free setup/restore pair. Such a
pair cannot safely recover from partial initialization or early returns.

### Handovers

Resume, New Session, login, and terminal commands follow this sequence:

```text
suspend TerminalSession
-> run child synchronously
-> resume the same TerminalSession
-> drain pending input
-> rescan/redraw as required
```

Keyboard enhancement and bracketed paste must be disabled before the child and
restored only when s7s regains terminal ownership.

### Panic and process limits

- The panic hook uses the recorded active-mode mask to restore modes before the
  default hook prints.
- `Drop` remains a fallback for unwinding and failed explicit cleanup.
- No in-process mechanism can recover terminal state after `SIGKILL`, `abort`,
  power loss, or an unhandled terminating signal. `reset` is the documented
  manual recovery command.

## Verification map

| Behavior | Automated coverage |
| --- | --- |
| Grapheme movement/deletion/insertion | `ui/components/input.rs` tests |
| Whole-value selection: replace, clear, collapse to either edge | `ui/components/input.rs` tests + `ui/new_session/tests.rs` |
| Grapheme wrapping/truncation/sanitization | `ui/components/text.rs` tests |
| Exhaustive paste routing and no-submit behavior | `ui/paste.rs` and feature tests |
| Partial setup, cleanup ordering, retry, panic-mask restoration | `runtime::terminal_lifecycle_tests` |

The baseline suite is insufficient for terminal ownership changes. Run the real
terminal and PTY checks in
[testing.md](./testing.md#terminal-lifecycle-and-paste-checks), including:

- paste into every editable field;
- multiline paste into terminal-command mode;
- normal exit and every agent handover;
- kitty keyboard enhancement and legacy/tmux fallback;
- `S7S_PANIC_PROBE=1` panic restoration;
- verification that no mode leaks into the child or parent shell.

## Known physical limits

- A terminal without bracketed-paste support may deliver paste as key events;
  crossterm exposes no reliable capability query.
- A terminal or multiplexer may split one physical paste into multiple paste
  events. s7s guarantees that each received paste event is treated as text, not
  that upstream chunking never occurs.
- Legacy encoding cannot distinguish `ctrl+shift+n` from `ctrl+n`; New Session
  with Context remains available through Quick Command.
