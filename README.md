# bimbumbam

[![CI](https://github.com/gdmsl/bimbumbam/actions/workflows/ci.yml/badge.svg)](https://github.com/gdmsl/bimbumbam/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange.svg)

> A toddler-friendly fullscreen keyboard basher for Wayland.

`bimbumbam` covers every connected display with a colourful overlay that
turns every key press into a small visual celebration — letters, fireworks,
shockwaves, flying polygons, bouncing balls. The overlay grabs the keyboard
exclusively and (where the compositor supports it) inhibits compositor key
bindings, so a toddler can have their way with the keyboard without
launching apps, killing windows, switching workspaces, or otherwise harming
the parent's session.

A short three-key chord held for three seconds gets the parent out.

```
+-----------------------------------------------------------+
|                                                           |
|                            A                              |
|       *                                  *                |
|             *      .   *                                  |
|                  ★      .       ●                         |
|                                                           |
|                                                           |
+-----------------------------------------------------------+
                press any key to play  ·  Ctrl+Alt+Q to quit
```

## Features

- **Fullscreen on every display.** One `wlr-layer-shell` overlay surface
  per output, anchored to all four edges, exclusive zone disabled so panels
  don't show through.
- **Captures every key.** `KeyboardInteractivity::Exclusive` plus
  `zwp_keyboard_shortcuts_inhibit_v1` on supporting compositors — the
  toddler cannot alt-tab, switch workspaces, or trigger compositor shortcuts.
- **GPU-accelerated rendering.** A single wgpu pipeline with premultiplied
  alpha, runs comfortably at 60 fps on integrated GPUs.
- **Glyphon-based text.** Crisp scalable letters and digits.
- **Pleasant audio.** Each key plays a soft note from a C-major pentatonic
  scale — any combination is consonant. Toggle with `--mute`.
- **Toddler-resistant exit.** `Ctrl + Alt + Q`, all three held continuously
  for three seconds. The chord requires the modifiers to already be held
  when `Q` is pressed (a flat-handed slap won't register).
- **Calm-mode flag.** `--no-flash` removes the soft full-screen tints for
  light-sensitive viewers.

## Compositor support

`bimbumbam` requires a Wayland compositor that implements
[wlr-layer-shell](https://wayland.app/protocols/wlr-layer-shell-unstable-v1).

| Compositor          | layer-shell | shortcuts-inhibit | Notes                                          |
| ------------------- | :---------: | :---------------: | ---------------------------------------------- |
| **Sway**            | yes         | yes               | Full support.                                  |
| **niri**            | yes         | yes               | Full support.                                  |
| **Hyprland**        | yes         | partial           | Some hard-coded compositor binds may persist.  |
| **KDE Plasma 6**    | yes         | yes               | Full support.                                  |
| **River**           | yes         | yes               | Full support.                                  |
| **Wayfire**         | yes         | yes               | Full support.                                  |
| GNOME mutter        | **no**      | n/a               | Unsupported — no layer-shell.                  |

A compositor that does not implement keyboard-shortcuts-inhibit will still
run the app — but its own keybindings (e.g. `Super+Space` for a launcher)
will continue to fire.

### Verifying the inhibitor on niri / Sway

The protocol is advisory: a compositor may decline to inhibit specific
binds. niri exposes this as `allow-inhibiting=false` on individual binds in
`~/.config/niri/config.kdl` — most importantly, niri's default config marks
the inhibit-toggle key (`Mod+Escape`) that way. **Any bind in your config
with `allow-inhibiting=false` will fire even while bimbumbam is running.**

To diagnose what your compositor is doing, run with debug logging:

```sh
RUST_LOG=bimbumbam=info bimbumbam
```

You should see exactly one line per output of the form
`attached keyboard-shortcuts-inhibitor to layer surface`, followed by
`inhibitor ACTIVE — compositor shortcuts suppressed for our surface`. If
you see `inhibitor INACTIVE` or no `ACTIVE` line at all, the compositor is
choosing not to inhibit and the only sure-fire fix is to remove
`allow-inhibiting=false` from the offending bind in your compositor config.

For an absolute guarantee that no key reaches the compositor, the right
primitive is `ext-session-lock-v1` (the protocol screen-lockers use).
That's a planned `--lock` mode for a future release.

## Installation

### NixOS / Nix

```sh
# run once, no install
nix run github:gdmsl/bimbumbam

# install into the current-user profile
nix profile install github:gdmsl/bimbumbam

# build from a clone
nix build && ./result/bin/bimbumbam
```

The root `flake.nix` exposes `packages.default`, `apps.default`, and a
`devShells.default` with the Rust toolchain and every system library
pre-wired (Wayland, libxkbcommon, fontconfig, freetype, vulkan-loader,
ALSA). Reference it as a flake input from your NixOS or home-manager
configuration to add `bimbumbam` declaratively.

### Other Linux distributions

Build dependencies (Wayland, libxkbcommon, fontconfig + freetype, Vulkan
loader, ALSA development headers, and a Rust toolchain ≥ 1.85):

| Distro       | Package install                                                                              |
| ------------ | -------------------------------------------------------------------------------------------- |
| **Arch**     | `sudo pacman -S wayland libxkbcommon fontconfig vulkan-icd-loader alsa-lib pkgconf rustup`   |
| **Debian/Ubuntu** | `sudo apt install libwayland-dev libxkbcommon-dev libfontconfig-dev libfreetype-dev libvulkan-dev libasound2-dev pkg-config` |
| **Fedora**   | `sudo dnf install wayland-devel libxkbcommon-devel fontconfig-devel freetype-devel vulkan-loader-devel alsa-lib-devel pkgconf` |

Then:

```sh
cargo install --git https://github.com/gdmsl/bimbumbam --locked
bimbumbam
```

## Screenshots

> _A demo GIF will live at `docs/demo.gif`. Until then: imagine an explosion of
> colorful letters every time someone presses a key._

## Usage

```
bimbumbam [--mute] [--no-flash] [--volume FLOAT]
bimbumbam --help | --version
```

| Flag             | Effect                                                                |
| ---------------- | --------------------------------------------------------------------- |
| `--mute`         | Disable all sound.                                                    |
| `--no-flash`     | Disable the soft full-screen tints (calmer for sensitive viewers).    |
| `--volume FLOAT` | Volume multiplier in `[0.0, 1.0]` (default `1.0`).                    |

Logging is structured via [`tracing`](https://crates.io/crates/tracing). Set
`RUST_LOG=bimbumbam=debug` (or `=trace`) to follow surface lifecycle, key
events, and renderer fall-backs; the default filter is `bimbumbam=info,warn`.
A panic hook routes panic payloads through the same layer so a crash report
includes the location.

### Controls

| Input          | Effect                              |
| -------------- | ----------------------------------- |
| Letters        | Big bouncy letter + particles + chime |
| Digits         | Big digit + particles               |
| Space          | Firework + soft flash               |
| Enter          | Rainbow shockwave + chord           |
| Arrow keys     | Flying polygon                      |
| Anything else  | Random shape, spiral, or burst      |
| `Ctrl+Shift+S` | Start a 3 s countdown, then save a PNG of the clean frame |

### Exiting

Hold **Ctrl + Alt + Q** for **three seconds**. A red progress bar in the
top-right confirms the chord is being recognised. Releasing any of the three
keys resets the timer. The hint also re-fades in at the bottom of the screen
every 30 s so you don't have to remember it cold.

### Saving a screenshot

Press **Ctrl + Shift + S** to start a 3-second countdown (`3 → 2 → 1`). At
zero, bimbumbam captures the *clean* frame (no overlays) of the canonical
canvas, encodes it as PNG on a background thread, and shows a `Saved →
<path>` toast for ~2 s. Files land in `$XDG_PICTURES_DIR`,
`$HOME/Pictures` if it exists, or the current working directory, named
`bimbumbam-<unix-timestamp>.png`.

## How it works

```
src/
├── color.rs     — palette, premultiplied alpha, HSL → RGB
├── particle.rs  — short-lived sprite physics
├── effect.rs    — high-level effects + spawn routines
├── render.rs    — CPU-side draw batching
├── text.rs      — glyphon wrapper
├── gpu.rs       — wgpu pipeline + per-frame render
├── audio.rs     — rodio-backed pentatonic synth
├── keys.rs      — keysym classification + exit-gate state machine
├── config.rs    — argv parsing
├── wayland.rs   — Wayland event loop, layer-shell surfaces, key inhibit
├── lib.rs       — module declarations
└── main.rs      — entry point
```

The Wayland event loop is driven by [`calloop`](https://crates.io/crates/calloop)
with a 16 ms timer that calls `App::tick`. Each tick advances the
simulation, builds geometry into a single `DrawBatch`, and renders it once
per output (zoom-fit). The GPU pipeline is one shader, one render pass per
surface, premultiplied-alpha "over" compositing.

## Development

```sh
cargo test            # 30 unit tests covering pure logic
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run             # inside a Wayland session
```

CI runs the full suite on every push (see `.github/workflows/ci.yml`).

## Releasing

1. Bump `version` in `Cargo.toml`.
2. Move the `## [Unreleased]` block at the top of `CHANGELOG.md` under a new
   versioned heading and update the date.
3. Commit and tag: `git tag -s vX.Y.Z -m "vX.Y.Z"`.
4. Push: `git push --follow-tags`. The release workflow builds the binary,
   uploads a tarball, and generates release notes.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the dev loop, commit-message
conventions, and the AI-assistance disclosure rule.

## License

Released under the [MIT license](LICENSE).

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this project shall be licensed as above,
without any additional terms or conditions.
