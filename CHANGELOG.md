# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0] - 2026-06-25

### Changed

- **Gentler, instrument-like notes.** Each key now plays a struck
  music-box/glockenspiel voice — harmonic partials under a fast-attack,
  decaying envelope — instead of a constant-amplitude sine. The envelope
  starts and ends at silence, removing the click/pop at the start and end
  of every note, and the timbre is warmer and less robotic. Per-note
  levels and the simultaneous-note cap were lowered so fast mashing stays
  soft rather than clipping, and Space now plays a low, round note instead
  of the piercing top of the scale.

### Added

- **Periodic exit-hint reminder.** The "hold Ctrl+Alt+Q for 3 s to exit"
  cue now fades in at the bottom of the screen every 30 s for 3 s, so a
  parent walking up to a running session always finds it discoverable.
- **Screenshot mode.** `Ctrl+Shift+S` starts a 3-second on-screen
  countdown, then captures the clean canvas (no overlays) via an
  offscreen wgpu render + buffer readback, encodes it as PNG on a
  background thread, and displays a `Saved → <path>` toast. Output goes
  to `$XDG_PICTURES_DIR` / `$HOME/Pictures` / cwd as
  `bimbumbam-<unix-timestamp>.png`.

## [0.6.0] - 2026-05-09

Initial public release.

### Highlights

- Fullscreen `wlr-layer-shell` overlay across every connected output, rendered
  with a single GPU-batched `wgpu` pipeline.
- `zwp_keyboard_shortcuts_inhibit_v1` integration so compositor key bindings
  (workspace switches, launchers, kill-window) cannot reach the toddler on
  niri, Sway, KDE Plasma 6, River, and Wayfire.
- Toddler-resistant exit chord: `Ctrl + Alt + Q` held for 3 seconds, with a
  press-transition rule so a flat-handed slap does not register. Splash
  screen always shows the chord hint.
- Pleasant audio: per-key deterministic pentatonic notes via `rodio`,
  single-sink mixed chord on Enter, `--mute` and `--volume FLOAT` flags.
- Calm-mode flag (`--no-flash`) caps soft full-screen tints to remove any
  strobe risk for sensitive viewers.
- Hot-plug-safe: per-output surfaces are created and torn down cleanly;
  `OutputSurface` field order guarantees `wgpu::Surface` is dropped before
  the underlying `wl_surface`.
- 12 modules under `src/`, 36 unit tests covering color math, particle
  physics, draw batching, key state machine, and CLI parsing.
- GitHub Actions CI (rustfmt, clippy `-D warnings`, build, test, MSRV 1.85,
  `cargo-audit`) plus a tag-triggered release workflow.
- MIT licensed.
