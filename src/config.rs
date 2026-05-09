//! Command-line configuration parsing.
//!
//! Tiny, hand-rolled parser — argv is short enough that pulling in `clap`
//! would more than double our compile time for no readability win.

use std::process::ExitCode;

const HELP: &str = "\
bimbumbam — a toddler-friendly keyboard basher.

Usage:
    bimbumbam [--mute] [--no-flash] [--volume FLOAT]
    bimbumbam --help | --version

Options:
    --mute            Disable all sound.
    --no-flash        Disable soft full-screen flashes (calmer for sensitive viewers).
    --volume FLOAT    Volume multiplier in [0.0, 1.0]. Default 1.0.
    -h, --help        Show this help and exit.
    -V, --version     Show version and exit.

Controls:
    Any key       Spawn a colorful effect.
    Letters       Render the corresponding big letter + a deterministic note.
    Digits 0-9    Render the digit.
    Space         Burst of fireworks.
    Enter         Rainbow chime.
    Arrow keys    Send a flying shape.

To exit:
    Hold Ctrl + Alt + Q for 3 seconds. A red progress bar in the top-right
    confirms the parent is intentionally quitting.
";

#[derive(Clone, Debug)]
pub struct Config {
    pub mute: bool,
    pub no_flash: bool,
    pub volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mute: false,
            no_flash: false,
            volume: 1.0,
        }
    }
}

#[derive(Debug)]
pub enum ParseOutcome {
    Run(Config),
    /// Exit immediately with this code (help/version requested, or argv error).
    Exit(ExitCode),
}

impl Config {
    pub fn parse_argv<I, S>(args: I) -> ParseOutcome
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut cfg = Config::default();
        let mut iter = args.into_iter();
        // Skip program name.
        let _ = iter.next();
        while let Some(arg) = iter.next() {
            match arg.as_ref() {
                "--mute" => cfg.mute = true,
                "--no-flash" => cfg.no_flash = true,
                "--volume" => {
                    let Some(val) = iter.next() else {
                        eprintln!("--volume requires an argument\n\n{HELP}");
                        return ParseOutcome::Exit(ExitCode::FAILURE);
                    };
                    let Ok(parsed) = val.as_ref().parse::<f32>() else {
                        eprintln!(
                            "--volume must be a number, got '{}'\n\n{HELP}",
                            val.as_ref()
                        );
                        return ParseOutcome::Exit(ExitCode::FAILURE);
                    };
                    if !(0.0..=1.0).contains(&parsed) {
                        eprintln!("--volume must be in [0.0, 1.0], got {parsed}\n\n{HELP}");
                        return ParseOutcome::Exit(ExitCode::FAILURE);
                    }
                    cfg.volume = parsed;
                }
                "-h" | "--help" => {
                    print!("{HELP}");
                    return ParseOutcome::Exit(ExitCode::SUCCESS);
                }
                "-V" | "--version" => {
                    println!("bimbumbam {}", env!("CARGO_PKG_VERSION"));
                    return ParseOutcome::Exit(ExitCode::SUCCESS);
                }
                other => {
                    eprintln!("unknown argument: {other}\n\n{HELP}");
                    return ParseOutcome::Exit(ExitCode::FAILURE);
                }
            }
        }
        ParseOutcome::Run(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Option<Config> {
        match Config::parse_argv(std::iter::once("bimbumbam").chain(args.iter().copied())) {
            ParseOutcome::Run(c) => Some(c),
            ParseOutcome::Exit(_) => None,
        }
    }

    #[test]
    fn defaults_have_audio_and_flash() {
        let c = run(&[]).unwrap();
        assert!(!c.mute && !c.no_flash);
    }

    #[test]
    fn mute_flag_parses() {
        assert!(run(&["--mute"]).unwrap().mute);
    }

    #[test]
    fn no_flash_flag_parses() {
        assert!(run(&["--no-flash"]).unwrap().no_flash);
    }

    #[test]
    fn unknown_flag_returns_exit() {
        assert!(run(&["--bogus"]).is_none());
    }

    #[test]
    fn volume_in_range_parses() {
        assert!((run(&["--volume", "0.5"]).unwrap().volume - 0.5).abs() < 1e-6);
    }

    #[test]
    fn volume_out_of_range_rejected() {
        assert!(run(&["--volume", "1.5"]).is_none());
        assert!(run(&["--volume", "-0.1"]).is_none());
    }

    #[test]
    fn volume_non_number_rejected() {
        assert!(run(&["--volume", "abc"]).is_none());
    }

    #[test]
    fn volume_missing_argument_rejected() {
        assert!(run(&["--volume"]).is_none());
    }
}
