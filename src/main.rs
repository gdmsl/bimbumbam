use std::process::ExitCode;

use bimbumbam::config::{Config, ParseOutcome};
use bimbumbam::wayland::App;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() -> ExitCode {
    install_tracing();
    install_panic_hook();

    let config = match Config::parse_argv(std::env::args()) {
        ParseOutcome::Run(c) => c,
        ParseOutcome::Exit(code) => return code,
    };

    if let Err(err) = App::run(config) {
        // anyhow's Debug formatter prints the error chain.
        tracing::error!("{err:?}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Initialise `tracing` with an env-filtered fmt layer to stderr. Default
/// filter is `bimbumbam=info,warn` so the app is quiet by default; override
/// with `RUST_LOG=bimbumbam=debug` etc.
fn install_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("bimbumbam=info,warn"));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_writer(std::io::stderr);
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}

/// On panic, log the message + location through `tracing` so it is captured
/// alongside other errors, then chain to the default hook so the process
/// still aborts visibly.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "<unknown>".to_owned(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let payload = info
            .payload()
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        tracing::error!(location = %location, "fatal panic: {payload}");
        default_hook(info);
    }));
}
