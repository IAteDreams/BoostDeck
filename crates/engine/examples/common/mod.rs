//! Helpers shared by the examples: argument parsing, the routing journal
//! and dB formatting. Each example uses a different subset.

#![allow(dead_code, reason = "each example uses only some of these")]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use engine::routing::RoutingJournal;

pub use engine::console::{catch_ctrl_c, interrupted};

/// Removes `--gain-db <dB>` from the arguments, wherever it is, and returns
/// the gain: 0 dB when absent. Refuses anything outside the slider's range
/// rather than quietly clamping it.
pub fn take_gain_db(args: &mut Vec<String>) -> Result<f32> {
    let Some(flag) = args.iter().position(|arg| arg == "--gain-db") else {
        return Ok(0.0);
    };
    args.remove(flag);
    if flag >= args.len() {
        bail!("--gain-db needs a value in dB");
    }
    let value = args.remove(flag);
    let gain_db: f32 = value
        .parse()
        .with_context(|| format!("--gain-db {value:?} is not a number"))?;
    if !(dsp::GAIN_MIN_DB..=dsp::GAIN_MAX_DB).contains(&gain_db) {
        bail!(
            "--gain-db {gain_db} is outside {} to +{} dB",
            dsp::GAIN_MIN_DB,
            dsp::GAIN_MAX_DB
        );
    }
    Ok(gain_db)
}

/// Removes `flag` from the arguments, wherever it is, and says whether it was
/// there.
pub fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|arg| arg != flag);
    args.len() != before
}

/// The routing record in the app's `settings.json`: the two fields the app's
/// launch check reads. The app owns that file's schema, so every other field
/// is carried through untouched.
pub struct Journal {
    pub path: PathBuf,
}

impl Journal {
    pub fn new() -> Result<Self> {
        let (qualifier, organisation, application) = presets::PROJECT_DIRS;
        let dirs = directories::ProjectDirs::from(qualifier, organisation, application)
            .context("Windows did not report an app-data folder")?;
        Ok(Self {
            path: dirs.config_dir().join("settings.json"),
        })
    }

    fn read(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(serde_json::Value::Object(map)) => Ok(map),
                _ => bail!("{} is not a settings object", self.path.display()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", self.path.display())),
        }
    }
}

impl RoutingJournal for Journal {
    fn saved_original(&self) -> Option<String> {
        let settings = self
            .read()
            .inspect_err(|error| eprintln!("Ignoring the routing record: {error:#}"))
            .ok()?;
        if settings.get("routing_active") != Some(&serde_json::Value::Bool(true)) {
            return None;
        }
        settings
            .get("original_default_device_id")?
            .as_str()
            .map(str::to_owned)
    }

    /// Atomic, like the app's own save.
    fn record(&mut self, original: Option<&str>) -> Result<()> {
        let mut settings = self.read()?;
        settings.insert("routing_active".to_owned(), original.is_some().into());
        settings.insert("original_default_device_id".to_owned(), original.into());
        write_atomically(&self.path, &serde_json::to_string_pretty(&settings)?)
    }
}

fn write_atomically(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let temp = path.with_extension("json.tmp");
    let mut file =
        std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("writing {}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

/// Peak as dBFS, which is the scale the limiter and the meters work in.
pub fn dbfs(peak: f32) -> String {
    if peak <= 0.0 {
        return "  -inf".to_owned();
    }
    format!("{:>6.1}", 20.0 * peak.log10())
}
