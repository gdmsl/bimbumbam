# Contributing to bimbumbam

Thanks for your interest. This is a small portfolio project, but contributions
that fix real bugs, broaden compositor compatibility, or sharpen the code are
welcome.

## Dev loop

The Nix flake bundles every system dependency. From a clone:

```sh
nix develop          # drops you into a shell with cargo + libs wired
cargo build          # debug build
cargo test --lib     # unit tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Without Nix, install the system packages listed in the
[README](README.md#other-linux-distributions) and use any Rust ≥ 1.85.

The app needs a real Wayland session that supports `wlr-layer-shell` to run
end-to-end (Sway, Hyprland, niri, KDE Plasma 6, river, Wayfire). Headless CI
exercises everything that doesn't require a display.

## Commit conventions

- **[Conventional Commits](https://www.conventionalcommits.org/)**: prefix
  subjects with `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `ci:`, etc.
- Subject under ~70 characters; body wraps at 72.
- Keep the change atomic — one logical concern per commit.

## AI-assistance disclosure

If you used an AI coding assistant on a non-trivial part of a patch, follow
the Linux kernel's
[`Documentation/process/coding-assistants.rst`](https://www.kernel.org/doc/html/latest/process/coding-assistants.html)
convention: add a trailer naming the model, e.g.

```
Assisted-by: Claude:claude-opus-4-7
```

The human author still owns the patch and the `Signed-off-by` line (if you
add one). AI assistance does not absolve a contributor of reviewing the
output.

## Bug reports

Please include:
- Compositor and version (e.g. `niri 25.10`, `sway 1.10`, `hyprland 0.45`).
- Whether `bimbumbam` warned about missing
  `zwp_keyboard_shortcuts_inhibit_manager_v1` at startup.
- A `RUST_LOG=bimbumbam=debug` log around the failure.
- GPU adapter (`vulkaninfo --summary` or similar) if the issue is render-related.

## Licensing

By submitting a contribution, you agree to license it under the
[MIT license](LICENSE).
