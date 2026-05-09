//! Keyboard state machine. Lives separately from the Wayland event handlers
//! to keep the policy testable.

use std::time::{Duration, Instant};

use xkbcommon::xkb::keysyms;

/// Total time the parent must hold the exit chord. Long enough that no toddler
/// palm-mash will land on it, short enough that the parent doesn't think it's
/// stuck.
pub const EXIT_HOLD_DURATION: Duration = Duration::from_secs(3);

/// Hard cap on the keypress queue. Prevents an unattended held key (auto-repeat)
/// from accumulating during the splash screen, between frames, or if the
/// renderer stalls.
pub const PENDING_QUEUE_CAP: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Letter(char),
    Space,
    Enter,
    Arrow(ArrowDir),
    Misc(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowDir {
    Up,
    Down,
    Left,
    Right,
}

impl ArrowDir {
    pub fn vector(self) -> (f32, f32) {
        match self {
            Self::Up => (0.0, -1.0),
            Self::Down => (0.0, 1.0),
            Self::Left => (-1.0, 0.0),
            Self::Right => (1.0, 0.0),
        }
    }
}

/// Tracks modifier and exit-key state. We treat the exit chord as a *transition*:
/// the parent must press the exit key while Ctrl+Alt are already held — keys
/// already down at startup do not count, and releasing any of the three
/// resets the timer.
#[derive(Default)]
pub struct ExitGate {
    pub ctrl: bool,
    pub alt: bool,
    pub exit_key: bool,
    started: Option<Instant>,
}

impl ExitGate {
    pub fn reset(&mut self) {
        self.ctrl = false;
        self.alt = false;
        self.exit_key = false;
        self.started = None;
    }

    pub fn set_modifiers(&mut self, ctrl: bool, alt: bool) {
        if !ctrl || !alt {
            self.started = None;
        }
        self.ctrl = ctrl;
        self.alt = alt;
    }

    /// Notify a press of the exit key. Only starts the timer if the modifiers
    /// were *already* held — stops a flat-handed slap from registering.
    pub fn press_exit_key(&mut self, now: Instant) {
        if self.ctrl && self.alt && !self.exit_key {
            self.started = Some(now);
        }
        self.exit_key = true;
    }

    pub fn release_exit_key(&mut self) {
        self.exit_key = false;
        self.started = None;
    }

    /// Returns `(should_exit, progress 0..=1)`.
    pub fn poll(&self, now: Instant) -> (bool, f32) {
        let Some(started) = self.started else {
            return (false, 0.0);
        };
        if !self.ctrl || !self.alt || !self.exit_key {
            return (false, 0.0);
        }
        let held = now.saturating_duration_since(started);
        let progress = (held.as_secs_f32() / EXIT_HOLD_DURATION.as_secs_f32()).clamp(0.0, 1.0);
        (held >= EXIT_HOLD_DURATION, progress)
    }
}

/// Translate an X keysym into one of our high-level [`KeyAction`]s, or `None`
/// when the press is consumed by the exit chord and shouldn't trigger an effect.
pub fn classify(keysym: u32) -> KeyAction {
    match keysym {
        keysyms::KEY_space => KeyAction::Space,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => KeyAction::Enter,
        keysyms::KEY_Up => KeyAction::Arrow(ArrowDir::Up),
        keysyms::KEY_Down => KeyAction::Arrow(ArrowDir::Down),
        keysyms::KEY_Left => KeyAction::Arrow(ArrowDir::Left),
        keysyms::KEY_Right => KeyAction::Arrow(ArrowDir::Right),
        // Letters: render uppercase regardless of shift state so the screen
        // shows what a child recognizes.
        s if (keysyms::KEY_a..=keysyms::KEY_z).contains(&s) => {
            KeyAction::Letter((s as u8 - b'a' + b'A') as char)
        }
        s if (keysyms::KEY_A..=keysyms::KEY_Z).contains(&s) => KeyAction::Letter(s as u8 as char),
        s if (keysyms::KEY_0..=keysyms::KEY_9).contains(&s) => KeyAction::Letter(s as u8 as char),
        other => KeyAction::Misc(other),
    }
}

/// The keysym used as the third leg of the exit chord. Chosen to be unlikely
/// in toddler-mashing patterns and conventional ("Q for quit").
pub const EXIT_KEYSYM: u32 = keysyms::KEY_q;

/// The keysym used as the third leg of the screenshot chord (Ctrl + Shift + S).
/// Adding `Shift` differentiates it from the toddler-friendly default
/// behaviour of bare letter keys.
pub const SCREENSHOT_KEYSYM: u32 = keysyms::KEY_s;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_letters() {
        assert_eq!(classify(keysyms::KEY_a), KeyAction::Letter('A'));
        assert_eq!(classify(keysyms::KEY_z), KeyAction::Letter('Z'));
        assert_eq!(classify(keysyms::KEY_A), KeyAction::Letter('A'));
        assert_eq!(classify(keysyms::KEY_5), KeyAction::Letter('5'));
    }

    #[test]
    fn classifies_special() {
        assert_eq!(classify(keysyms::KEY_space), KeyAction::Space);
        assert_eq!(classify(keysyms::KEY_Return), KeyAction::Enter);
        assert_eq!(classify(keysyms::KEY_Up), KeyAction::Arrow(ArrowDir::Up));
    }

    #[test]
    fn classifies_misc_for_unknown() {
        match classify(0xfffd) {
            KeyAction::Misc(_) => {}
            other => panic!("expected Misc, got {other:?}"),
        }
    }

    #[test]
    fn exit_gate_requires_modifiers_first() {
        let mut g = ExitGate::default();
        let t0 = Instant::now();
        // Press exit key alone — should not start
        g.press_exit_key(t0);
        let (exit, prog) = g.poll(t0 + EXIT_HOLD_DURATION);
        assert!(!exit && prog == 0.0);
    }

    #[test]
    fn exit_gate_starts_only_after_modifiers() {
        let mut g = ExitGate::default();
        let t0 = Instant::now();
        g.set_modifiers(true, true);
        g.press_exit_key(t0);
        let (exit, prog) = g.poll(t0 + EXIT_HOLD_DURATION);
        assert!(exit);
        assert!((prog - 1.0).abs() < 1e-3);
    }

    #[test]
    fn releasing_exit_key_resets_timer() {
        let mut g = ExitGate::default();
        let t0 = Instant::now();
        g.set_modifiers(true, true);
        g.press_exit_key(t0);
        g.release_exit_key();
        let (exit, _) = g.poll(t0 + EXIT_HOLD_DURATION);
        assert!(!exit);
    }

    #[test]
    fn dropping_a_modifier_resets_timer() {
        let mut g = ExitGate::default();
        let t0 = Instant::now();
        g.set_modifiers(true, true);
        g.press_exit_key(t0);
        g.set_modifiers(true, false); // alt released
        let (exit, _) = g.poll(t0 + EXIT_HOLD_DURATION);
        assert!(!exit);
    }
}
