use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::config;

/// Initialises file-backed logging.
///
/// A TUI must never write to stdout, so logs go to a daily-rotated file under
/// the XDG state directory. The returned guard must live for the duration of
/// the process.
pub fn init() -> Result<WorkerGuard> {
    let dir = config::state_dir().context("could not determine the XDG state directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("could not create log directory {}", dir.display()))?;

    let appender = tracing_appender::rolling::daily(&dir, "ascolta.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .with_env_filter(filter)
        .init();

    tracing::info!(directory = %dir.display(), "logging initialised");
    Ok(guard)
}
