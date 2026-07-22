# Repository-wide Refactoring Plan

> **Status: Proposed (in progress).** Work packages R0–R7, R8a, R8b, and R9 are implemented:
> the `Changes` log is archived in [development-history.md](./development-history.md),
> `AGENTS.md` is slimmed to routing/rules/verification, [architecture.md](./architecture.md)
> is the current-state map, [testing.md](./testing.md) holds the verification
> matrix, the Rust toolchain is pinned (`rust-toolchain.toml`),
> `scripts/check.sh` is the canonical check, the app is now a library crate
> (`src/lib.rs` + `src/runtime.rs`) with `main.rs` reduced to a thin entry shim,
> shared UI primitives are extracted into `src/ui/components/` (input, modal,
> scrollbar, text) — R5, the New Session feature is extracted into
> `src/ui/new_session/` (state, input, render) — R6, and the Profile feature is
> extracted into `src/ui/profile/` (state, input, render) — R7, the Detail screen is
> extracted into `src/ui/detail/` (state, input, render) — R8a, and the Session
> screen (table/filter/preview + `/` keyword prompt) is extracted into
> `src/ui/session/` (state, input, render) — R8b, and the remaining overlays
> (agent/folder filters, rename/delete confirmations, the reusable message alert,
> help, and theme selection) are extracted into `src/ui/overlays/`
> (`filters`/`confirm`/`message`/`help`/`theme`, one file each) with Quick
> Command's rendering moved into `src/ui/quick.rs` — R9 (all behavior-preserving
> moves; effect-based decoupling per §8 is deferred). R10 is split into two
> packages: **R10a** extracts the in-place external effects into an explicit
> `AppEffect` / `App::apply_effect` boundary (`src/ui/effect.rs`) — key handlers
> enqueue `RefreshAll` / `RenameSession` / `DeleteSession` / `ProfileSaved`
> instead of performing rename/delete/rescan/persist inline (done; the first
> behavior-structure change, no async runtime per §8.2). **R10b** (background-job
> coordination — isolating the usage/model receivers into a `BackgroundState`)
> and R11 onward remain proposed.

## 1. Status and Purpose

This document proposes a staged refactoring of the s7s codebase, repository
instructions, and supporting documentation. It is a planning document, not an
authorization to perform every phase as one change.

The primary objective is to reduce the amount of context an agent or maintainer
must load before making a safe change, while preserving the behavior and
compatibility knowledge accumulated during development.

The plan covers:

- repository instructions and historical change records;
- architecture and documentation routing;
- Rust module boundaries and application state ownership;
- terminal process probing and external side effects;
- list and detailed session parsing;
- test, toolchain, and manual verification contracts.

## 2. Current Assessment

The project is functionally healthy but has several concentration points that
increase change cost.

### 2.1 Strengths to Preserve

- Agent-specific storage parsers are separated by source format.
- Session context has a neutral model shared by the TUI, CLI, and handoff
  exporter.
- Sensitive context is redacted before excerpting.
- Cache compatibility is explicit and versioned.
- High-risk external CLI behavior has dedicated probe and manual verification
  procedures.
- Unit coverage is broad, and real-data turn parity is available for parser
  changes.
- Domain documents already capture many current contracts.

### 2.2 Main Sources of Friction

| Area | Current issue | Consequence |
| --- | --- | --- |
| `AGENTS.md` | Chronological change records dominate the file | Every agent session receives large amounts of unrelated historical context |
| `src/ui/mod.rs` | Global state, transitions, validation, persistence, and key handling are concentrated in one module | Small UI changes require broad code reading and create merge conflicts |
| `src/ui/render.rs` | All screens, dialogs, components, and render tests are concentrated in one module | Visual changes have a large search surface and weak feature ownership |
| `App` | UI state and external effects are represented together | Key handlers can perform file, scan, rename, and background-job operations directly |
| `usage.rs` | PTY driving, process discovery, login checks, and usage parsing coexist | Model discovery depends on a feature-specific module for generic terminal probing |
| List/context parsers | Some active-turn and rollback rules are implemented twice | A storage-format change can produce list/detail parity regressions |
| Tooling | No repository-pinned Rust toolchain or single canonical check entry point | Different agents can receive different Clippy and build results |
| Documentation | Current contracts, implementation history, and completed plans overlap | Agents must distinguish current truth from historical explanation |

### 2.3 Baseline Snapshot

At the time this plan was written:

- Rust sources contain approximately 23,800 lines.
- `src/ui/` contains approximately 44% of those lines, including tests.
- `src/ui/mod.rs` and `src/ui/render.rs` together contain approximately 9,700
  lines.
- `AGENTS.md` is approximately 88 KB, of which roughly 96% is the chronological
  `Changes` section.
- `cargo test -q` passes 280 tests with one ignored test.
- strict Clippy on the current unpinned toolchain reports warnings as errors,
  demonstrating that the repository does not yet have a reproducible lint
  baseline.

These numbers are diagnostic baselines, not permanent quality gates. File size
alone does not justify a split; ownership and change isolation do.

## 3. Goals

### 3.1 Agent and Maintainer Efficiency

- Make the correct entry point for a change discoverable without reading the
  entire repository history.
- Keep mandatory instructions short, current, and enforceable.
- Make each UI feature readable through a bounded set of state, input, render,
  and test files.
- Separate pure decisions from file-system, process, terminal, and persistence
  effects.
- Provide one canonical command for routine automated verification.

### 3.2 Safety

- Preserve CLI syntax, storage compatibility, cache behavior, TUI interaction,
  and visual layout unless a phase explicitly approves a behavior change.
- Preserve all external CLI verification requirements.
- Keep list/detail turn parity and rewind/backtrack behavior intact.
- Avoid introducing unredacted session data into caches, diagnostics, or test
  fixtures.
- Keep all committed source, comments, documentation, and commit messages in
  English.

### 3.3 Long-term Structure

- Establish stable feature boundaries rather than only creating smaller files.
- Keep domain policy independent from ratatui and crossterm where practical.
- Make application effects explicit and testable without starting real agent
  processes.
- Share low-level transcript interpretation without forcing lightweight list
  indexing to construct full session context.

## 4. Non-goals

This refactoring must not be used as an opportunity to make unrelated product
changes.

- No TUI redesign or shortcut changes.
- No changes to public CLI commands or output contracts.
- No new agent type.
- No storage migration unless separately approved and documented.
- No cache schema change merely to support code movement.
- No replacement of ratatui, crossterm, serde, clap, or rusqlite.
- No trait hierarchy solely to remove exhaustive `Agent` matches.
- No complete rewrite of session parsers.
- No deletion of historical design evidence before it is preserved or distilled.
- No mass test relocation that separates tests from the behavior they verify.

## 5. Refactoring Principles

### 5.1 Structural Commits Must Preserve Behavior

Each structural change should be independently reviewable. File moves, symbol
renames, and behavior changes should not be mixed unless separation is
impractical and explicitly justified.

### 5.2 Extract by Feature Ownership

Do not split a large file into arbitrary line-count chunks. A feature module
should own a coherent group of:

- state;
- input handling and transitions;
- validation and pure decisions;
- rendering;
- tests.

Shared widgets and primitives should be extracted only after at least two
features demonstrate the same contract.

### 5.3 Keep Effects at Boundaries

State transitions should describe requested work. Boundary code should perform
the work and return a result. Tests should be able to verify transitions without
touching user configuration or starting a CLI.

### 5.4 Current Contracts Beat Chronology

Mandatory agent instructions should state what is true now. Historical reasons
belong in domain documentation, focused decision records, Git history, or the
archived development history.

### 5.5 Optimize Parser Sharing for Correctness and Cost

List indexing must remain lightweight. Parser refactoring should share decoded
events and active-path reducers, not force all scans through the full context
model.

## 6. Target Repository Information Architecture

### 6.1 `AGENTS.md`

`AGENTS.md` should contain only information that must be applied to nearly every
relevant task:

1. A concise repository purpose statement.
2. A change-area-to-document routing table.
3. Security, privacy, language, and compatibility rules.
4. A verification matrix keyed by change area.
5. A short list of current, unresolved hazards that cannot yet live in a domain
   document.

The chronological `Changes` section should not remain in automatically loaded
instructions. A soft size budget of 5-10 KB is appropriate; exceeding it should
trigger review, not automatic truncation.

### 6.2 Historical Preservation

Before shortening `AGENTS.md`:

- copy the existing `Changes` section verbatim into
  `docs/development-history.md`;
- mark it as historical and non-authoritative;
- preserve dates, iteration numbers, verification evidence, and resolved notes;
- do not instruct agents to read it by default;
- retain Git history as the authoritative record of exact code changes.

After preservation, distill still-current rules into the appropriate domain
documents. Do not leave a rule only in the archive.

### 6.3 Architecture Map

Create `docs/architecture.md` as a compact current-state map, not as a narrative
tour or README feature chapter. It should include:

- process entry points and execution modes;
- session scan and cache flow;
- list parsing versus detailed context parsing;
- TUI state/event/effect/render flow;
- usage and model probe flow;
- persistence files and ownership;
- a table mapping common change requests to source modules and required tests.

The document should link to detailed domain documents instead of duplicating
their contracts.

### 6.4 Domain Documents

Existing domain documents should be reviewed for current-state authority:

| Document | Intended ownership |
| --- | --- |
| `docs/session-title-compat.md` | Title sources, precedence, rename paths, and verified CLI behavior |
| `docs/usage-display.md` | Usage semantics, parsing, probe behavior, and rendering states |
| `docs/models.md` | Model discovery, cache semantics, defaults, and last selection |
| `docs/profiles.md` | Profile identity, environment injection, persistence, and related UI behavior |
| `docs/session-context.md` | Context model, parser parity, CLI projections, and contextual launch |
| `docs/testing.md` | Canonical automated and manual verification matrix |
| UI style documents | Reusable visual and focus contracts only |

Chronological statements should be rewritten as current contracts unless their
date is essential evidence of an external CLI version.

### 6.5 Decision Records

Use a short architecture decision record only for non-obvious decisions that are
likely to be challenged again and cannot be explained clearly in a domain
document. Examples include:

- why list parsing remains lighter than detailed context parsing;
- why Antigravity extra profiles cannot inject an environment root;
- why external rename success requires storage verification.

Do not create decision records for routine implementation details.

### 6.6 Completed Plans

Implementation plans should be marked `Implemented`, `Superseded`, or `Proposed`
at the top. A completed plan is historical context and must not compete with the
current domain document as the source of truth.

## 7. Target Code Architecture

The exact file names may change during implementation, but the ownership model
should remain stable.

```text
src/
  main.rs                  CLI startup and terminal lifecycle only
  lib.rs                   testable application modules
  app/
    mod.rs                 App composition and top-level state
    event.rs               global event routing
    effect.rs              external work requested by state transitions
    background.rs          usage/model job coordination
  ui/
    mod.rs                 shared UI exports
    components/
      input.rs
      modal.rs
      scrollbar.rs
      text.rs
    session/
      state.rs
      input.rs
      render.rs
    detail/
      state.rs
      input.rs
      render.rs
    profile/
      state.rs
      input.rs
      render.rs
    new_session/
      state.rs
      input.rs
      render.rs
    quick/
      state.rs
      input.rs
      render.rs
    overlays/
      filters.rs
      confirm.rs
      message.rs
      help.rs
      theme.rs
  probe/
    mod.rs                 shared probe result and execution contracts
    pty.rs                 PTY lifecycle and screen capture
    process.rs             process discovery and termination helpers
    usage.rs               usage-specific commands and parsing
    models.rs              model-specific commands and parsing
  parser/
    mod.rs
    common.rs              shared cleanup and indexing helpers
    claude/
      events.rs            raw record decoding and active-branch reduction
      index.rs             lightweight Session construction
      context.rs           detailed ContextTurn construction
    codex/
      events.rs            raw record decoding and rollback reduction
      index.rs
      context.rs
    antigravity/
      index.rs
      context.rs
  session_context/
    model.rs
    excerpt.rs
    redact.rs
    render.rs
    resolve.rs
```

This is a direction, not a requirement to create every directory immediately.
The implementation should stop splitting when ownership is already clear.

## 8. Application State and Effect Model

### 8.1 State Ownership

Replace the flat collection of modal and screen fields in `App` gradually with
feature-owned state.

```rust
struct App {
    data: AppData,
    navigation: NavigationState,
    session_ui: SessionUi,
    profile_ui: ProfileUi,
    overlay: Option<OverlayState>,
    background: BackgroundState,
    pending_effect: Option<AppEffect>,
}
```

This example is illustrative. The important constraints are:

- mutually exclusive overlays should not require many independent `Option`
  fields plus a separate mode enum;
- feature state should own its cursor, focus, scroll, and draft data;
- rendering should receive the narrowest state required;
- tests should construct a feature fixture without initializing unrelated
  profiles, models, themes, and histories.

### 8.2 Effects

Key handlers should return or enqueue an explicit effect for work such as:

- rescan sessions;
- save profiles or model selection;
- create a directory;
- rename or delete session artifacts;
- start usage or model probes;
- hand over the terminal to an agent or shell command;
- persist theme or command history.

Effect execution remains synchronous or threaded according to current behavior.
This phase does not require an async runtime.

### 8.3 Transition Safety

Every extracted screen or dialog must retain tests for:

- opening and closing behavior;
- focus and cursor movement;
- confirm and cancel transitions;
- preservation or reset of draft state;
- emitted effects;
- failure messages and return modes.

## 9. UI Refactoring Sequence

### 9.1 Shared Primitives

Extract only stable, low-risk primitives first:

- Unicode-safe text input editing and cursor movement;
- modal frame and button styles;
- width-aware truncation and wrapping;
- persistent list viewport/scrollbar behavior.

These extractions must retain existing render-buffer and interaction tests.

### 9.2 New Session Feature

Extract New Session first because it has the densest combination of profile,
model, folder, context, project-directory confirmation, focus, and launch logic.

The feature boundary should own:

- `NewSessionState`, focus, model options, and source context;
- folder matching and dropdown ordering;
- validation and project-directory confirmation decisions;
- launched model selection persistence request;
- key handling and rendering;
- focused unit and render tests.

The feature should depend on small read-only catalogs rather than the complete
`App` where practical.

### 9.3 Profile Feature

Extract profile table, profile form, deletion confirmation, config-directory
confirmation, shortcut assignment, and login request creation. Keep profile
persistence and CLI login execution behind effects.

### 9.4 Session and Detail Features

Extract session table/filter/preview and detailed turn navigation. Preserve the
existing relationship between selected indices and the canonical session vector
until a separate change proves a safer identity-based selection model.

### 9.5 Remaining Overlays

Extract filters, rename/delete confirmations, help, message, and theme overlays.
Quick Command may remain a separate module but should adopt the same state/input/
render boundary.

### 9.6 Render Tests

Move render tests with their feature modules. Keep shared component tests next to
the component. Retain a small number of full-frame integration tests for:

- the main Session screen;
- Profile screen;
- Detail screen;
- one representative modal with backdrop dimming;
- New Session with Context.

## 10. Probe and Process Refactoring

### 10.1 Shared PTY Driver

Move generic terminal behavior out of `usage.rs`:

- PTY creation and sizing;
- command spawning and environment injection;
- input writes and screen stabilization;
- timeout and descendant-process cleanup;
- vt100 screen reconstruction;
- optional diagnostic screen dumping.

The shared driver must not know usage labels or model syntax.

### 10.2 Feature-specific Probes

Usage and model modules should own only:

- agent-specific commands and navigation input;
- login and availability interpretation;
- screen-to-domain parsing;
- fallback and cache policy;
- probe-specific tests and fixtures.

### 10.3 Process Safety

Before moving code, capture tests or seams for:

- timeout cleanup;
- descendant process termination;
- environment sanitization;
- missing executable behavior;
- missing profile directories;
- demo-mode prohibition of real CLI execution.

## 11. Parser Refactoring

Parser work is the highest-risk phase and should begin only after the lower-risk
repository and UI work is stable.

### 11.1 Claude

Create one decoder for the record fields needed by both consumers:

- UUID and parent UUID;
- user, assistant, boundary, tool, and title events;
- sidechain and task-notification identity;
- readable text payloads.

Keep one active-branch reducer. Feed its output into:

- a lightweight index accumulator that creates `Session`;
- a detailed accumulator that creates `ContextTurn` values.

### 11.2 Codex

Create one decoder for:

- session metadata;
- user and assistant messages from both supported record forms;
- tool calls/results and reasoning entries;
- `thread_rolled_back` boundaries.

Keep one rollback boundary reducer while allowing the index and context
accumulators to retain different payload detail.

### 11.3 Antigravity

Do not force the SQLite index path and JSONL context path into a false shared
format. Share only session identity, transcript resolution, cleanup, and
turn-normalization behavior that is truly common.

### 11.4 Performance and Compatibility Gates

For every parser phase:

- compare session count, order, IDs, titles, Q counts, and search blobs before
  and after on representative real data;
- run real-data turn parity;
- inspect rewind/backtrack storage diffs when those rules are touched;
- retain cache freshness semantics and displayed mtimes;
- avoid a cache version bump unless serialized meaning changes;
- record scan-time and cache-size comparisons when event allocation changes.

## 12. Toolchain and Verification Refactoring

### 12.1 Rust Version Decision

Choose and document one of these policies before adding a toolchain file:

- pin the exact Rust version used for releases; or
- declare a minimum supported Rust version and run CI against both minimum and
  stable.

Do not silently pin the current local version without deciding the public support
policy.

### 12.2 Canonical Check Entry Point

Add a portable repository script, for example `scripts/check.sh`, that runs the
routine automated contract:

```text
cargo fmt --all -- --check
cargo test -q
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

The script should fail fast, print the failed stage clearly, and contain no local
paths. Expensive real-data and real-CLI checks remain explicit opt-in steps.

### 12.3 CI

Add CI only after the toolchain policy and canonical check command are agreed.
CI should run on macOS because terminal, path, and storage behavior are central to
the product. A Linux compile check may be added if Linux is intended to remain a
supported build target.

### 12.4 Test Layers

| Layer | Purpose | Typical trigger |
| --- | --- | --- |
| Pure unit tests | Parsing, validation, transitions, formatting | Every change |
| Render-buffer tests | Layout and style contracts | UI changes |
| Filesystem fixture tests | Cache, scan, rename, demo, persistence | Storage changes |
| Real-data parity | List/detail active-turn agreement | Parser turn-selection changes |
| Probe checks | Match actual external CLI screens | Usage/model parser or CLI upgrade |
| Manual CLI checks | Rename, rewind, prompt injection, handover | Relevant compatibility changes |
| PTY TUI checks | Keyboard protocol and visual interaction | Layout/input protocol changes |

`docs/testing.md` should become the authoritative matrix and `AGENTS.md` should
contain only its concise routing rules.

## 13. Phased Execution Plan

### Phase 0 — Baseline and Decision Gates

Deliverables:

- capture the current automated-test result and strict-Clippy findings;
- record current source sizes and major module responsibilities;
- decide the Rust support policy;
- decide whether the full `Changes` archive remains committed indefinitely;
- agree on behavior-preserving commit boundaries.

Exit criteria:

- no application behavior has changed;
- all unresolved policy decisions needed by Phase 1 and Phase 2 are explicit.

### Phase 1 — Instructions and Documentation

Deliverables:

- preserve `Changes` in `docs/development-history.md`;
- distill current contracts into domain documents;
- replace chronological `AGENTS.md` content with routing, rules, and verification;
- add the compact architecture map;
- mark completed implementation plans appropriately;
- remove duplicated or stale statements.

Exit criteria:

- no current safety or manual-verification rule exists only in the archive;
- `AGENTS.md` contains no chronological implementation log;
- a maintainer can locate the relevant source and test contract from the
  architecture map and routing table;
- documentation contains no personal paths or private data.

### Phase 2 — Reproducible Tooling and Library Boundary

Deliverables:

- add the agreed Rust toolchain/MSRV policy;
- resolve the strict-Clippy baseline;
- add the canonical check script;
- introduce `lib.rs` and move testable modules behind it while keeping `main.rs`
  behavior unchanged;
- optionally add CI after the local command is stable.

Exit criteria:

- the canonical check passes from a clean checkout;
- CLI help and application startup remain unchanged;
- `main.rs` owns startup and terminal lifecycle rather than domain logic.

### Phase 3 — UI Primitives and New Session

Deliverables:

- extract text input and stable modal primitives;
- extract New Session state, input, render, and tests;
- represent directory creation and launch as effects instead of direct key-handler
  side effects.

Exit criteria:

- all existing New Session tests pass without semantic changes;
- `cargo build --release` succeeds;
- manual TUI checks confirm layout, dropdowns, focus rotation, context launch
  presentation, and cancel/confirm behavior.

### Phase 4 — Profile, Session, Detail, and Overlays

Deliverables:

- extract remaining feature modules;
- reduce `App` to composition and cross-feature coordination;
- move feature render tests with their owners;
- keep a small full-frame integration suite.

Exit criteria:

- no feature handler requires the full render module;
- external operations are represented as effects;
- keyboard and visual contracts remain unchanged;
- release build and relevant PTY checks pass.

### Phase 5 — Background Jobs and Probes

Deliverables:

- isolate background receiver coordination;
- extract the shared PTY/process driver;
- make usage and model probes independent clients of that driver;
- preserve demo-mode isolation.

Exit criteria:

- usage and model tests pass;
- `--usage-probe` and `--model-probe` match actual CLI screens;
- no real CLI is spawned in demo mode or unit tests;
- process cleanup behavior is verified.

### Phase 6 — Parser Event Layers

Deliverables:

- introduce shared Claude event decoding and active-branch reduction;
- introduce shared Codex event decoding and rollback reduction;
- retain separate lightweight index and detailed context accumulators;
- share only genuine Antigravity common behavior.

Exit criteria:

- all parser fixtures pass;
- real-data turn parity reports zero mismatches;
- real rewind/backtrack verification passes where applicable;
- session ordering, title resolution, Q counts, and search behavior match the
  baseline;
- scan performance does not regress materially without explicit approval.

### Phase 7 — Cleanup and Final Audit

Deliverables:

- remove obsolete adapters, dead compatibility names, and temporary re-exports;
- review public visibility and module documentation;
- update architecture and domain documents to final paths;
- audit comments and documents for duplicated or stale explanations;
- run the complete automated and manual verification matrix.

Exit criteria:

- no temporary migration layer remains without a documented reason and removal
  condition;
- repository instructions and documentation match the final code structure;
- all required checks pass;
- the worktree contains no generated or personal artifacts.

## 14. Work Package Breakdown

The phases should be implemented as small work packages rather than one long-lived
branch.

| ID | Work package | Depends on | Risk |
| --- | --- | --- | --- |
| R0 | Baseline and policy decisions | None | Low |
| R1 | Archive and slim `AGENTS.md` | R0 | Medium |
| R2 | Current-state architecture and document cleanup | R1 | Low |
| R3 | Toolchain, strict Clippy baseline, check script | R0 | Low |
| R4 | `lib.rs` and thin `main.rs` boundary | R3 | Medium |
| R5 | Shared UI input/modal primitives | R4 | Medium |
| R6 | New Session feature extraction | R5 | High |
| R7 | Profile feature extraction | R5 | Medium |
| R8a | Detail feature extraction (`ui/detail/`) — done | R5 | High |
| R8b | Session table/filter/preview extraction (`ui/session/`) — done | R5 | High |
| R9 | Overlay and render-test redistribution (`ui/overlays/`) — done | R6-R8 | Medium |
| R10a | App in-place effects (`AppEffect`/`apply_effect`) — done | R6-R9 | High |
| R10b | Background job coordination (`BackgroundState`) | R6-R9 | High |
| R11 | Generic PTY/process probe layer | R3, R10 | High |
| R12 | Claude normalized events | R3 | High |
| R13 | Codex normalized events | R3 | High |
| R14 | Antigravity parser boundary review | R12-R13 | High |
| R15 | Final cleanup and documentation audit | R2-R14 | Medium |

R12 and R13 may proceed independently after the baseline is stable, but they
should not be combined into one review.

## 15. Risk Register

### 15.1 Accidental Behavior Changes During File Splits

Mitigation:

- preserve public function signatures initially;
- move tests with code before redesigning interfaces;
- compare render buffers and CLI output;
- keep move-only commits separate from cleanup commits.

### 15.2 Rust Borrowing Pressure Produces Worse Abstractions

Splitting `App` can encourage excessive cloning, `Rc<RefCell<_>>`, or broad mutable
borrows.

Mitigation:

- prefer explicit transition inputs and returned effects;
- keep immutable catalogs separate from mutable feature drafts;
- accept a temporary coordinator method rather than adding interior mutability
  solely to satisfy a desired file layout.

### 15.3 Parser Unification Increases Scan Cost

Mitigation:

- share decoded event shapes and reducers, not complete context construction;
- benchmark rebuild and cache-hit paths;
- avoid storing detailed tool payloads in list-parser events.

### 15.4 Historical Knowledge Is Lost

Mitigation:

- archive before deletion;
- distill every still-current invariant into a domain document;
- retain iteration identifiers in the archive for Git and session lookup;
- review removed `AGENTS.md` statements against the verification matrix.

### 15.5 Documentation Becomes Another Parallel Source of Truth

Mitigation:

- give every contract one owning document;
- use links instead of copied prose;
- state whether each plan is proposed, implemented, or superseded;
- update code and its owning document in the same change.

### 15.6 Large Refactoring Blocks Product Work

Mitigation:

- merge work packages independently;
- stop after any phase that delivers sufficient improvement;
- allow urgent fixes against the current structure;
- avoid a long-lived repository-wide refactoring branch.

## 16. Review Checklist for Each Work Package

Before implementation:

- identify whether the package changes structure, behavior, or both;
- read the routed domain documents;
- list affected persistence and external CLI contracts;
- establish the exact before/after comparison.

During implementation:

- preserve unrelated user changes;
- avoid local paths and real session content in fixtures;
- keep comments focused on non-obvious agent-relevant constraints;
- update tests and owning documentation together;
- do not add compatibility aliases without a removal condition.

Before completion:

- run formatting, tests, strict Clippy, and release build;
- run all change-area-specific manual checks;
- inspect `git diff --check` and the complete diff;
- confirm no generated, secret, or personal artifacts are tracked;
- record remaining work in the owning document, not as an ever-growing
  chronological instruction entry.

## 17. Overall Completion Criteria

The repository-wide refactoring is complete when:

- mandatory agent instructions contain only current routing and safety rules;
- historical changes remain available without occupying default agent context;
- architecture and testing documents identify current ownership and validation;
- UI features have bounded state/input/render/test ownership;
- `App` coordinates features instead of implementing all of them;
- external side effects are explicit and independently testable;
- usage and model probing share a neutral PTY/process layer;
- Claude and Codex list/context parsers share active-path semantics without making
  scans construct full context;
- the toolchain and routine verification command are reproducible;
- public CLI, TUI behavior, stored data, cache semantics, and compatibility checks
  remain intact;
- no known regression or undocumented temporary migration layer remains.

## 18. Recommended Starting Point

Begin with R0-R3. They reduce recurring agent overhead, preserve historical
knowledge, and establish a reproducible baseline without restructuring runtime
behavior. Proceed to the UI phases only after those changes are merged and the
baseline is stable. Treat parser event-layer work as a separate high-risk effort,
not as a prerequisite for UI cleanup.
