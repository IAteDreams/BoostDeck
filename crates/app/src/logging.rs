//! Log files under `%LOCALAPPDATA%\BoostDeck\logs`.
//!
//! One file per run, named by its start time, and only the newest
//! `KEEP_FILES` are kept. Debug builds also log to their console; release
//! builds have none, so the file is the only record of what happened - which
//! is also why panics are written to it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use simplelog::{
    ColorChoice, CombinedLogger, Config, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};

/// How many log files to keep, this run's included.
const KEEP_FILES: usize = 5;

const PREFIX: &str = "boostdeck-";
const SUFFIX: &str = ".log";

/// Starts logging to a new file, and to the console in debug builds.
/// Returns the file's path, if one could be made. Never fails the app: a
/// read-only profile still gets a working BoostDeck, just without a log.
pub fn init() -> Option<PathBuf> {
    let config = config();
    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();
    if cfg!(debug_assertions) {
        loggers.push(TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ));
    }
    let path = log_dir().and_then(|dir| open_new(&dir).ok());
    if let Some((path, file)) = &path {
        loggers.push(WriteLogger::new(
            LevelFilter::Info,
            config,
            file.try_clone().ok()?,
        ));
        if let Some(dir) = path.parent() {
            prune(dir, KEEP_FILES);
        }
    }
    let _ = CombinedLogger::init(loggers);
    install_panic_logger();
    path.map(|(path, _)| path)
}

/// Times in local time, so a log lines up with the clock the user saw. Must
/// run before any other thread starts: the offset cannot be read safely after.
fn config() -> Config {
    let mut builder = ConfigBuilder::new();
    if builder.set_time_offset_to_local().is_err() {
        eprintln!("Logging in UTC: the local time offset is unavailable");
    }
    builder.build()
}

/// `%LOCALAPPDATA%\BoostDeck\logs`.
fn log_dir() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    Some(base.data_local_dir().join("BoostDeck").join("logs"))
}

/// Creates `boostdeck-<unix millis>.log`. Millisecond names sort in start
/// order, which is what pruning relies on.
fn open_new(dir: &Path) -> std::io::Result<(PathBuf, fs::File)> {
    fs::create_dir_all(dir)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{PREFIX}{millis:013}{SUFFIX}"));
    let file = fs::File::create(&path)?;
    Ok((path, file))
}

/// Deletes all but the newest `keep` log files in `dir`. Other files there
/// are left alone.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(SUFFIX))
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        if let Err(error) = fs::remove_file(old) {
            log::warn!("Could not remove old log {}: {error}", old.display());
        }
    }
}

/// Writes a panic's message and location to the log before the default
/// hook prints it to a console that a release build does not have.
fn install_panic_logger() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Panic: {info}");
        log::logger().flush();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest_logs_and_nothing_else_is_touched() {
        let dir = std::env::temp_dir().join(format!("boostdeck-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for millis in [1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000] {
            fs::write(dir.join(format!("{PREFIX}{millis:013}{SUFFIX}")), "x").unwrap();
        }
        fs::write(dir.join("notes.txt"), "keep me").unwrap();

        prune(&dir, 5);

        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "boostdeck-0000000003000.log",
                "boostdeck-0000000004000.log",
                "boostdeck-0000000005000.log",
                "boostdeck-0000000006000.log",
                "boostdeck-0000000007000.log",
                "notes.txt",
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
