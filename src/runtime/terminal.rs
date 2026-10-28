//! Terminal lifecycle: the modes s7s turns on while it owns the screen, their balanced restore, and the panic hook.

use super::*;

/// A terminal state s7s turns on while the TUI owns the screen. Each one must be
/// turned off again before a child process or the parent shell sees the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalMode {
    Raw,
    AlternateScreen,
    BracketedPaste,
    KeyboardEnhancement,
}

impl TerminalMode {
    fn name(self) -> &'static str {
        match self {
            TerminalMode::Raw => "raw mode",
            TerminalMode::AlternateScreen => "alternate screen",
            TerminalMode::BracketedPaste => "bracketed paste",
            TerminalMode::KeyboardEnhancement => "keyboard enhancement flags",
        }
    }

    fn bit(self) -> u8 {
        match self {
            TerminalMode::Raw => 1,
            TerminalMode::AlternateScreen => 1 << 1,
            TerminalMode::BracketedPaste => 1 << 2,
            TerminalMode::KeyboardEnhancement => 1 << 3,
        }
    }
}

/// Low-level terminal state operations. Abstracted so the lifecycle policy in
/// [`TerminalModes`] — ordering, flag bookkeeping, and failure recovery — is unit
/// testable with injected failures instead of only against a real terminal.
pub(super) trait TerminalOps {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()>;
    fn disable(&mut self, mode: TerminalMode) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    /// Whether the terminal speaks the kitty keyboard enhancement protocol.
    fn supports_keyboard_enhancement(&mut self) -> bool;
}

/// Modes currently enabled by [`CrosstermOps`], mirrored into a global so the
/// panic hook can restore the terminal. `Drop` cannot serve that purpose alone:
/// the runtime prints the panic message *before* unwinding runs destructors, so
/// the message would land on the alternate screen and be erased with it.
pub(super) static ACTIVE_MODES: AtomicU8 = AtomicU8::new(0);

/// Real terminal operations against `stdout`.
pub(super) struct CrosstermOps;

impl TerminalOps for CrosstermOps {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        // [`ACTIVE_MODES`] is the single source of truth for the real terminal, so
        // a redundant enable/disable is a no-op here. That matters after a panic:
        // the hook already restored the terminal, and the still-unwinding
        // `TerminalSession::drop` must not pop a second keyboard-enhancement level
        // — which would belong to whatever process launched s7s.
        if ACTIVE_MODES.load(Ordering::Relaxed) & mode.bit() != 0 {
            return Ok(());
        }
        let mut out = io::stdout();
        match mode {
            TerminalMode::Raw => enable_raw_mode()?,
            // App does not handle mouse events. Capturing mouse events intercepts default terminal
            // text selection, so we only activate the alternate screen.
            TerminalMode::AlternateScreen => execute!(out, EnterAlternateScreen)?,
            TerminalMode::BracketedPaste => execute!(out, EnableBracketedPaste)?,
            // Enhanced keyboard protocol (where supported): lets the terminal report
            // Ctrl+Shift+N distinctly from Ctrl+N (legacy encoding sends the same control
            // byte for both). DISAMBIGUATE_ESCAPE_CODES is sufficient and keeps plain
            // printable-key handling unchanged. Unsupported terminals keep legacy input;
            // the Quick Command palette is the functional fallback there.
            TerminalMode::KeyboardEnhancement => execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?,
        }
        ACTIVE_MODES.fetch_or(mode.bit(), Ordering::Relaxed);
        Ok(())
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if ACTIVE_MODES.load(Ordering::Relaxed) & mode.bit() == 0 {
            return Ok(());
        }
        let mut out = io::stdout();
        match mode {
            TerminalMode::Raw => disable_raw_mode()?,
            TerminalMode::AlternateScreen => execute!(out, LeaveAlternateScreen)?,
            TerminalMode::BracketedPaste => execute!(out, DisableBracketedPaste)?,
            TerminalMode::KeyboardEnhancement => execute!(out, PopKeyboardEnhancementFlags)?,
        }
        ACTIVE_MODES.fetch_and(!mode.bit(), Ordering::Relaxed);
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Show)
    }

    fn supports_keyboard_enhancement(&mut self) -> bool {
        keyboard_enhancement_supported()
    }
}

/// Whether the terminal supports the kitty keyboard enhancement protocol.
/// Queried once per process (the query needs raw mode and one terminal roundtrip);
/// re-entering the TUI after agent handovers reuses the cached answer.
pub(super) fn keyboard_enhancement_supported() -> bool {
    use std::sync::OnceLock;
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        matches!(
            crossterm::terminal::supports_keyboard_enhancement(),
            Ok(true)
        )
    })
}

/// Names the terminal mode in an I/O failure, so a cleanup error reaching the
/// user says which state could not be restored.
pub(super) fn named(mode: TerminalMode, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", mode.name()))
}

/// Ordered setup and teardown of the terminal modes, tracking exactly which ones
/// are currently on. Every enable records its success immediately, so a failure
/// halfway through setup only undoes what actually took effect.
pub(super) struct TerminalModes<O: TerminalOps> {
    ops: O,
    raw: bool,
    alternate: bool,
    bracketed_paste: bool,
    keyboard_enhanced: bool,
}

impl<O: TerminalOps> TerminalModes<O> {
    fn new(ops: O) -> Self {
        TerminalModes {
            ops,
            raw: false,
            alternate: false,
            bracketed_paste: false,
            keyboard_enhanced: false,
        }
    }

    /// Rebuilds the flag set from an [`ACTIVE_MODES`] bitmask, for restoring the
    /// terminal from a context that does not own the session (the panic hook).
    fn from_active_mask(ops: O, mask: u8) -> Self {
        let mut modes = TerminalModes::new(ops);
        for mode in [
            TerminalMode::Raw,
            TerminalMode::AlternateScreen,
            TerminalMode::BracketedPaste,
            TerminalMode::KeyboardEnhancement,
        ] {
            *modes.flag(mode) = mask & mode.bit() != 0;
        }
        modes
    }

    fn flag(&mut self, mode: TerminalMode) -> &mut bool {
        match mode {
            TerminalMode::Raw => &mut self.raw,
            TerminalMode::AlternateScreen => &mut self.alternate,
            TerminalMode::BracketedPaste => &mut self.bracketed_paste,
            TerminalMode::KeyboardEnhancement => &mut self.keyboard_enhanced,
        }
    }

    /// Enables one mode unless it is already on, recording success before returning.
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if *self.flag(mode) {
            return Ok(());
        }
        self.ops.enable(mode).map_err(|err| named(mode, err))?;
        *self.flag(mode) = true;
        Ok(())
    }

    /// Disables one mode, clearing its flag only on success (see [`Self::restore`]).
    /// Returns the failure so the caller can keep going and report the first one.
    fn disable(&mut self, mode: TerminalMode) -> Option<io::Error> {
        if !*self.flag(mode) {
            return None;
        }
        match self.ops.disable(mode) {
            Ok(()) => {
                *self.flag(mode) = false;
                None
            }
            Err(err) => Some(named(mode, err)),
        }
    }

    /// Turns on every mode the TUI needs. Raw mode goes first because the
    /// keyboard-enhancement capability query needs it. On failure the modes
    /// already enabled are turned back off before the error propagates, so a
    /// half-configured terminal is never handed back to the shell.
    ///
    /// Idempotent, which makes it double as the resume path after a handover.
    fn enter(&mut self) -> io::Result<()> {
        let steps = [
            TerminalMode::Raw,
            TerminalMode::AlternateScreen,
            TerminalMode::BracketedPaste,
        ];
        for mode in steps {
            if let Err(err) = self.enable(mode) {
                let _ = self.restore();
                return Err(err);
            }
        }
        // Optional feature: an unsupported or failing terminal keeps legacy input.
        if self.ops.supports_keyboard_enhancement() {
            let _ = self.enable(TerminalMode::KeyboardEnhancement);
        }
        Ok(())
    }

    /// Turns off every mode that is still on, in reverse-dependency order, and
    /// restores cursor visibility.
    ///
    /// Every applicable step is attempted even when an earlier one fails, and the
    /// first error is returned. Only successful steps clear their flag, so a later
    /// retry (an explicit call, or `Drop`) redoes exactly what is still active.
    fn restore(&mut self) -> io::Result<()> {
        let mut first_err: Option<io::Error> = None;
        // Keyboard enhancement and bracketed paste come off first: they must not
        // leak into a child agent CLI or the parent shell.
        for mode in [
            TerminalMode::KeyboardEnhancement,
            TerminalMode::BracketedPaste,
            TerminalMode::AlternateScreen,
        ] {
            if let Some(err) = self.disable(mode) {
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
        // Cursor visibility is restored after leaving the alternate screen: some
        // terminals track it per screen buffer, so showing it first can leave the
        // main screen with a hidden cursor. ratatui hides the cursor while drawing,
        // so this runs unconditionally rather than behind a mode flag.
        if let Err(err) = self.ops.show_cursor() {
            if first_err.is_none() {
                first_err = Some(err);
            }
        }
        if let Some(err) = self.disable(TerminalMode::Raw) {
            if first_err.is_none() {
                first_err = Some(err);
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

/// Owns the TUI terminal handle together with the modes enabled for it, so the
/// terminal is restored on every exit path — ordinary return, `?` propagation, or
/// an unwind panic.
///
/// `Drop` cannot cover `SIGKILL`, `process::abort`, `panic = "abort"`, power loss,
/// or an unhandled terminating signal (`SIGTERM`/`SIGHUP`); `reset` remains the
/// documented manual recovery.
pub(super) struct TerminalSession {
    terminal: Tui,
    modes: TerminalModes<CrosstermOps>,
}

impl TerminalSession {
    pub(super) fn enter() -> Result<Self> {
        let mut modes = TerminalModes::new(CrosstermOps);
        modes.enter()?;
        let terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => terminal,
            Err(err) => {
                // The modes are on but nothing owns them yet: undo them here.
                let _ = modes.restore();
                return Err(err.into());
            }
        };
        Ok(TerminalSession { terminal, modes })
    }

    pub(super) fn terminal_mut(&mut self) -> &mut Tui {
        &mut self.terminal
    }

    /// Releases the terminal for a child process (or for good).
    pub(super) fn suspend(&mut self) -> io::Result<()> {
        self.modes.restore()
    }

    /// Re-acquires the terminal after a child process exited and forces a full
    /// redraw. The clear runs only once every required mode is back on.
    pub(super) fn resume(&mut self) -> Result<()> {
        self.modes.enter()?;
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.modes.restore();
    }
}

/// Restores the terminal from the panic hook, before the panic message is printed.
///
/// The panicking thread does not own the [`TerminalSession`], so the modes are
/// reconstructed from [`ACTIVE_MODES`]. Doing this in the hook rather than relying
/// on `Drop` is what keeps the message readable: unwinding prints it first and
/// only then runs destructors, so a `Drop`-only restore would erase the message
/// together with the alternate screen.
pub(super) fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mask = ACTIVE_MODES.load(Ordering::Relaxed);
        if mask != 0 {
            let _ = TerminalModes::from_active_mask(CrosstermOps, mask).restore();
        }
        previous(info);
    }));
}

/// RAII raw-mode guard for the temporary prompts shown while the main
/// [`TerminalSession`] is suspended. Without it, an early return or a panic inside
/// a prompt would leave raw mode enabled on the parent shell.
///
/// Only valid while no session holds raw mode: the guard's `Drop` disables raw
/// mode unconditionally, so nesting it inside a live TUI session would break that
/// session's input.
pub(super) struct RawModeGuard;

impl RawModeGuard {
    pub(super) fn enter() -> Option<Self> {
        CrosstermOps
            .enable(TerminalMode::Raw)
            .ok()
            .map(|()| RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = CrosstermOps.disable(TerminalMode::Raw);
    }
}

#[cfg(test)]
mod tests;
