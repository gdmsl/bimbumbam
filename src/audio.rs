//! Tiny additive synth: turn a key press into a soft, pleasant note. Uses
//! [`rodio`] driven from a single output stream. Notes are picked from a major
//! pentatonic so any combination is consonant — important for a toddler
//! mashing the keyboard.
//!
//! The audio thread runs on rodio's own background thread; this module just
//! constructs sources and pushes them into a sink. We swallow audio errors —
//! audio is never load-bearing for the experience.

use std::time::Duration;

use rodio::source::{SineWave, Source};
use rodio::{OutputStream, OutputStreamHandle, Sink};

/// Pleasant, consonant scale spanning C3 → C7. Three octaves of major
/// pentatonic gives 16 distinct pitches — comfortably enough that letters and
/// digits map to unique notes most of the time, and any combination remains
/// consonant.
const PENTATONIC_HZ: &[f32] = &[
    130.81, 146.83, 164.81, 196.00, 220.00, // C3 D3 E3 G3 A3
    261.63, 293.66, 329.63, 392.00, 440.00, // C4 D4 E4 G4 A4
    523.25, 587.33, 659.25, 783.99, 880.00,  // C5 D5 E5 G5 A5
    1046.50, // C6
];

const NOTE_DURATION_MS: u64 = 320;
const NOTE_AMPLITUDE: f32 = 0.18;
const CHIME_AMPLITUDE: f32 = 0.12;
/// Hard cap on simultaneous notes. Auto-repeat on a held key can otherwise
/// stack sinks indefinitely.
const MAX_LIVE_SINKS: usize = 8;

pub struct Audio {
    /// Held to keep the device open. Dropping it would silence everything.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    /// Bag of sinks that have been spawned. We drain finished ones lazily on
    /// each play so we never accumulate them past the active note count.
    sinks: Vec<Sink>,
    volume: f32,
}

impl Audio {
    pub fn try_new(volume: f32) -> Option<Self> {
        // OutputStream::try_default fails on systems without a working PulseAudio /
        // PipeWire / ALSA setup; we degrade silently to a no-op.
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            handle,
            sinks: Vec::new(),
            volume: volume.clamp(0.0, 1.0),
        })
    }

    fn prepare_sink(&mut self) -> Option<Sink> {
        self.sinks.retain(|s| !s.empty());
        if self.sinks.len() >= MAX_LIVE_SINKS {
            // Drop the oldest live sink to make room — keeps the audio bounded
            // under sustained mashing without surprising the listener.
            self.sinks.remove(0).stop();
        }
        Sink::try_new(&self.handle).ok()
    }

    fn push(&mut self, sink: Sink) {
        self.sinks.push(sink);
    }

    /// Play a soft pentatonic note. `index` selects which pitch (modulo the
    /// scale length); using a deterministic index per key means the same key
    /// always sounds the same.
    pub fn play_note(&mut self, index: usize) {
        let Some(sink) = self.prepare_sink() else {
            return;
        };
        let hz = PENTATONIC_HZ[index % PENTATONIC_HZ.len()];
        let source = SineWave::new(hz)
            .take_duration(Duration::from_millis(NOTE_DURATION_MS))
            .amplify(NOTE_AMPLITUDE * self.volume)
            .fade_in(Duration::from_millis(8));
        sink.append(source);
        self.push(sink);
    }

    /// Play the rainbow chime — three notes mixed into a single sink so a
    /// chord counts as one entry against [`MAX_LIVE_SINKS`].
    pub fn play_chime(&mut self) {
        let Some(sink) = self.prepare_sink() else {
            return;
        };
        let n = |i: usize| {
            SineWave::new(PENTATONIC_HZ[i])
                .take_duration(Duration::from_millis(NOTE_DURATION_MS * 2))
                .amplify(CHIME_AMPLITUDE * self.volume)
                .fade_in(Duration::from_millis(20))
        };
        let chord = n(5).mix(n(7)).mix(n(9));
        sink.append(chord);
        self.push(sink);
    }
}

/// Map a printable letter or digit to a stable scale-degree. A→0, B→1, …,
/// Z→25; digits map to their value plus 1. Modulo of the pentatonic length
/// is applied at play time, so adjacent letters always sound adjacent.
pub fn pitch_index_for_char(ch: char) -> usize {
    let upper = ch.to_ascii_uppercase();
    if upper.is_ascii_alphabetic() {
        (upper as u8 - b'A') as usize
    } else if upper.is_ascii_digit() {
        (upper as u8 - b'0') as usize + 1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_pitch_distinct_for_neighbors() {
        assert_ne!(pitch_index_for_char('A'), pitch_index_for_char('B'));
        // Lower- and upper-case map to the same note.
        assert_eq!(pitch_index_for_char('a'), pitch_index_for_char('A'));
    }

    #[test]
    fn digit_pitch_is_monotonic_within_scale_octave() {
        let zero = pitch_index_for_char('0') % PENTATONIC_HZ.len();
        let one = pitch_index_for_char('1') % PENTATONIC_HZ.len();
        assert!(one > zero);
    }
}
