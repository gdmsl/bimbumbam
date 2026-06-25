//! Tiny struck-instrument synth: turn a key press into a soft, chimey note —
//! think music box / glockenspiel rather than a bare test tone. Driven from a
//! single [`rodio`] output stream. Notes are picked from a major pentatonic so
//! any combination is consonant — important for a toddler mashing the keyboard.
//!
//! Each note is a [`Voice`]: a handful of harmonic partials under a percussive
//! amplitude envelope (fast attack, exponential decay, short release to silence).
//! The envelope is what makes it sound *struck* instead of held, and — because
//! the signal starts and ends at exactly zero — it removes the clicks/pops you
//! get from chopping a constant-amplitude sine mid-cycle.
//!
//! The audio thread runs on rodio's own background thread; this module just
//! constructs sources and pushes them into a sink. We swallow audio errors —
//! audio is never load-bearing for the experience.

use std::f32::consts::TAU;
use std::time::Duration;

use rodio::source::Source;
use rodio::{OutputStream, OutputStreamHandle, Sink};

/// Pleasant, consonant scale spanning C3 → C6. Three octaves of major
/// pentatonic gives 16 distinct pitches — comfortably enough that letters and
/// digits map to unique notes most of the time, and any combination remains
/// consonant.
const PENTATONIC_HZ: &[f32] = &[
    130.81, 146.83, 164.81, 196.00, 220.00, // C3 D3 E3 G3 A3
    261.63, 293.66, 329.63, 392.00, 440.00, // C4 D4 E4 G4 A4
    523.25, 587.33, 659.25, 783.99, 880.00,  // C5 D5 E5 G5 A5
    1046.50, // C6
];

const SAMPLE_RATE: u32 = 48_000;
/// How long a single note rings out. The per-partial decay drops the body to a
/// near-silent tail well before this, so the value just sets how gently the note
/// fades, not how long it stays prominent.
const NOTE_DURATION_MS: u64 = 600;
/// Per-note levels are deliberately low: up to [`MAX_LIVE_SINKS`] notes can ring
/// at once, and the device sums them, so headroom here is what keeps a fast mash
/// from summing past unity and clipping into harshness.
const NOTE_AMPLITUDE: f32 = 0.14;
const CHIME_AMPLITUDE: f32 = 0.09;
/// Hard cap on simultaneous notes. Auto-repeat on a held key can otherwise
/// stack sinks indefinitely, and a smaller cap also bounds total loudness.
const MAX_LIVE_SINKS: usize = 6;

/// One overtone of a struck note: a frequency multiple of the fundamental, a
/// relative loudness, and how fast it decays. Real struck bars/bells let the
/// high partials die away quickly, leaving a pure-ish fundamental ringing —
/// that mellowing is most of what makes a tone read as an *instrument*.
struct Partial {
    /// Frequency relative to the fundamental. Near-integer keeps mashed keys
    /// consonant; the tiny offsets add a touch of bell-like shimmer.
    ratio: f32,
    /// Loudness relative to the fundamental.
    amp: f32,
    /// Exponential decay rate (per second). Higher = dies away sooner.
    decay: f32,
}

/// Partials for the voice, fundamental first. The fast-decaying upper partials
/// give the bright "ping" of the strike; once they fade, a warm harmonic core
/// rings on.
const PARTIALS: &[Partial] = &[
    Partial {
        ratio: 1.00,
        amp: 1.00,
        decay: 2.6,
    },
    Partial {
        ratio: 2.00,
        amp: 0.50,
        decay: 3.8,
    },
    Partial {
        ratio: 3.00,
        amp: 0.24,
        decay: 5.0,
    },
    Partial {
        ratio: 4.01,
        amp: 0.12,
        decay: 6.8,
    },
    Partial {
        ratio: 5.40,
        amp: 0.07,
        decay: 9.5,
    },
];

/// A single struck note: additive partials shaped by a percussive envelope.
/// Implements [`Source`] so it can be appended straight to a rodio [`Sink`].
struct Voice {
    sample: u32,
    total: u32,
    attack: u32,
    release_start: u32,
    base_hz: f32,
    gain: f32,
}

impl Voice {
    fn new(base_hz: f32, gain: f32) -> Self {
        let total = (SAMPLE_RATE as u64 * NOTE_DURATION_MS / 1000) as u32;
        // 4 ms attack removes the start click; 70 ms release guarantees the
        // waveform reaches zero before the cutoff, removing the stop click.
        let attack = SAMPLE_RATE * 4 / 1000;
        let release = SAMPLE_RATE * 70 / 1000;
        // The envelope math assumes the attack and release ramps don't overlap;
        // a too-short note would make them collide and dip the middle.
        debug_assert!(attack + release <= total, "note too short for its envelope");
        Self {
            sample: 0,
            total,
            attack,
            release_start: total.saturating_sub(release),
            base_hz,
            gain,
        }
    }

    /// Amplitude envelope at the current sample: linear attack, linear release
    /// to silence. The per-partial exponential decay shapes the body between.
    fn envelope(&self) -> f32 {
        let attack = if self.sample < self.attack {
            self.sample as f32 / self.attack as f32
        } else {
            1.0
        };
        let release = if self.sample >= self.release_start {
            (self.total - self.sample) as f32 / (self.total - self.release_start) as f32
        } else {
            1.0
        };
        attack * release
    }
}

impl Iterator for Voice {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.sample >= self.total {
            return None;
        }
        let t = self.sample as f32 / SAMPLE_RATE as f32;
        let mut s = 0.0;
        for p in PARTIALS {
            s += p.amp * (TAU * self.base_hz * p.ratio * t).sin() * (-p.decay * t).exp();
        }
        let out = (s * self.envelope() * self.gain).clamp(-1.0, 1.0);
        self.sample += 1;
        Some(out)
    }
}

impl Source for Voice {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(NOTE_DURATION_MS))
    }
}

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
            // Drop the oldest live note to make room, so the key just pressed
            // always sounds. The oldest is the most decayed, and per-note levels
            // are low, so the cut is quiet — but it is still an abrupt stop, so
            // the cap is kept small to make this path rare under fast mashing.
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
        sink.append(Voice::new(hz, NOTE_AMPLITUDE * self.volume));
        self.push(sink);
    }

    /// Play the rainbow chime — three notes mixed into a single sink so a
    /// chord counts as one entry against [`MAX_LIVE_SINKS`].
    pub fn play_chime(&mut self) {
        let Some(sink) = self.prepare_sink() else {
            return;
        };
        let gain = CHIME_AMPLITUDE * self.volume;
        let chord = Voice::new(PENTATONIC_HZ[5], gain)
            .mix(Voice::new(PENTATONIC_HZ[7], gain))
            .mix(Voice::new(PENTATONIC_HZ[9], gain));
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

    #[test]
    fn pitch_index_defaults_unknown_chars_to_first_note() {
        // Anything that isn't an ASCII letter/digit falls back to note 0 rather
        // than panicking — spaces, punctuation, accents, emoji.
        for ch in [' ', '!', 'é', 'ñ', '🎵'] {
            assert_eq!(pitch_index_for_char(ch), 0, "char {ch:?}");
        }
        assert_eq!(pitch_index_for_char('Z'), 25);
    }

    fn note_sample_count() -> usize {
        (SAMPLE_RATE as u64 * NOTE_DURATION_MS / 1000) as usize
    }

    #[test]
    fn voice_starts_and_ends_at_silence() {
        // The anti-click guarantee: the waveform has no jump at either edge.
        let mut v = Voice::new(440.0, NOTE_AMPLITUDE);
        let first = v.next().unwrap();
        assert!(first.abs() < 1e-4, "starts from silence: {first}");
        let last = v.last().unwrap();
        assert!(last.abs() < 1e-4, "ends at silence: {last}");
    }

    #[test]
    fn envelope_ramps_up_then_down() {
        // Attack rises monotonically to full, release falls monotonically to
        // silence — the shape that keeps the edges click-free.
        let mut v = Voice::new(440.0, NOTE_AMPLITUDE);
        let mut prev = 0.0;
        for s in 0..=v.attack {
            v.sample = s;
            let e = v.envelope();
            assert!(e >= prev, "attack dipped at sample {s}");
            prev = e;
        }
        assert!((prev - 1.0).abs() < 1e-6, "attack should reach full gain");
        let mut prev = 1.0;
        for s in v.release_start..v.total {
            v.sample = s;
            let e = v.envelope();
            assert!(e <= prev, "release rose at sample {s}");
            prev = e;
        }
    }

    #[test]
    fn voice_never_clips_at_playback_gain() {
        // The per-sample clamp is a safety net; at real volume no pitch should
        // ever reach it, or the tone would distort.
        for &hz in PENTATONIC_HZ {
            let peak = Voice::new(hz, NOTE_AMPLITUDE)
                .map(f32::abs)
                .fold(0.0_f32, f32::max);
            assert!(peak < 1.0, "{hz} Hz peaks at {peak}");
        }
    }

    #[test]
    fn chime_mix_is_full_length_and_unclipped() {
        let g = CHIME_AMPLITUDE;
        let chord = Voice::new(PENTATONIC_HZ[5], g)
            .mix(Voice::new(PENTATONIC_HZ[7], g))
            .mix(Voice::new(PENTATONIC_HZ[9], g));
        let samples: Vec<f32> = chord.collect();
        assert_eq!(
            samples.len(),
            note_sample_count(),
            "chord ends with its voices"
        );
        let peak = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
        assert!(peak < 1.0, "chord peaks at {peak}");
    }

    #[test]
    fn zero_gain_is_silent() {
        assert!(Voice::new(440.0, 0.0).all(|s| s == 0.0));
    }

    #[test]
    fn voice_is_deterministic() {
        // The same key must sound identical on every press.
        let a: Vec<f32> = Voice::new(440.0, NOTE_AMPLITUDE).collect();
        let b: Vec<f32> = Voice::new(440.0, NOTE_AMPLITUDE).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn voice_yields_expected_sample_count() {
        assert_eq!(
            Voice::new(440.0, NOTE_AMPLITUDE).count(),
            note_sample_count()
        );
    }
}
