//! Screenshot pipeline: countdown state, save-target resolution, PNG encoding.
//!
//! GPU readback happens in [`crate::gpu::Gpu::capture`]. This module owns the
//! state machine the UI consults to render the countdown / "saved" overlay,
//! plus the helper that turns RGBA bytes into a PNG file on a background
//! thread (the renderer never blocks on disk I/O).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Total countdown before the snap.
pub const COUNTDOWN: Duration = Duration::from_secs(3);
/// How long the "Saved → ..." toast remains on screen after the capture.
pub const TOAST: Duration = Duration::from_millis(2500);

#[derive(Debug)]
pub enum ScreenshotPhase {
    /// Counting down to capture. `started` is when the user pressed the chord.
    Counting { started: Instant },
    /// Capture has happened; saving in progress or already done.
    Saved { saved_at: Instant, path: PathBuf },
}

impl ScreenshotPhase {
    /// Number to display this frame, or `None` outside the countdown phase.
    /// `3.0..=2.0001` → "3", `2.0..=1.0001` → "2", `1.0..=0.0001` → "1",
    /// then `0.0..` → "smile!" briefly.
    pub fn countdown_label(&self, now: Instant) -> Option<&'static str> {
        let Self::Counting { started } = self else {
            return None;
        };
        let elapsed = now.saturating_duration_since(*started);
        if elapsed >= COUNTDOWN {
            return Some("smile!");
        }
        // We just checked `elapsed < COUNTDOWN`, but the lint can't see that.
        let remaining = COUNTDOWN.saturating_sub(elapsed);
        let secs = remaining.as_secs_f32().ceil() as u32;
        Some(match secs {
            3 => "3",
            2 => "2",
            _ => "1",
        })
    }

    /// Returns the saved path and how long the toast has been visible.
    pub fn saved(&self) -> Option<(&Path, Duration)> {
        if let Self::Saved { saved_at, path } = self {
            Some((path.as_path(), saved_at.elapsed()))
        } else {
            None
        }
    }
}

/// Resolve the screenshot output directory. Order:
/// 1. `$XDG_PICTURES_DIR` (Linux convention).
/// 2. `$HOME/Pictures` if it already exists.
/// 3. The current working directory.
pub fn resolve_dir() -> PathBuf {
    if let Ok(d) = std::env::var("XDG_PICTURES_DIR")
        && !d.is_empty()
    {
        return PathBuf::from(d);
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join("Pictures");
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from(".")
}

/// Build the destination filename for a fresh capture.
pub fn build_path(dir: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    dir.join(format!("bimbumbam-{stamp}.png"))
}

/// Encode an `RGBA8` buffer as PNG and write it to `path`. Blocking — call
/// this from a worker thread.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let buf = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(buf, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .context("failed to write PNG header")?;
    writer
        .write_image_data(rgba)
        .context("failed to write PNG body")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_label_progression() {
        let started = Instant::now();
        let phase = ScreenshotPhase::Counting { started };
        // At start: 3 seconds remaining → "3"
        assert_eq!(phase.countdown_label(started), Some("3"));
        // After 1.0s: ~2s remaining → "2"
        assert_eq!(
            phase.countdown_label(started + Duration::from_millis(1100)),
            Some("2")
        );
        // After 2.5s: ~0.5s remaining → "1"
        assert_eq!(
            phase.countdown_label(started + Duration::from_millis(2600)),
            Some("1")
        );
        // After 3.5s: countdown expired → "smile!"
        assert_eq!(
            phase.countdown_label(started + Duration::from_millis(3500)),
            Some("smile!")
        );
    }

    #[test]
    fn saved_phase_has_no_countdown_label() {
        let phase = ScreenshotPhase::Saved {
            saved_at: Instant::now(),
            path: PathBuf::from("/tmp/x.png"),
        };
        assert!(phase.countdown_label(Instant::now()).is_none());
    }

    #[test]
    fn build_path_is_inside_dir() {
        let p = build_path(Path::new("/tmp/here"));
        assert!(p.starts_with("/tmp/here"));
        assert!(p.extension().and_then(|s| s.to_str()) == Some("png"));
    }
}
