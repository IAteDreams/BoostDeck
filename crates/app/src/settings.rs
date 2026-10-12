//! App settings, stored as `settings.json` next to `presets.json` under
//! `%APPDATA%`.
//!
//! `routing_active` and `original_default_device_id` are what let the next
//! launch notice that the last run did not clean up and restore the user's
//! device. Saving is atomic - write a temp file, then rename it over the old
//! one - so a crash mid-save cannot leave a half-written file that loses the
//! original device.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, Result};
use engine::routing::RoutingJournal;
use serde::{Deserialize, Serialize};

/// Highest gain the slider offers unless the user unlocks it.
const DEFAULT_MAX_BOOST_DB: f32 = dsp::GAIN_MAX_DB;

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Clamp for the slider and for any gain a preset carries.
    pub max_boost_db: f32,

    /// Set by the settings checkbox behind the hearing-damage warning. Raises
    /// the clamp to `dsp::GAIN_MAX_DB_UNLOCKED`.
    pub unlock_max_boost: bool,

    /// How much audio to keep waiting in the ring, within
    /// `dsp::drift::RING_TARGET_MS_RANGE`.
    pub ring_target_ms: u32,

    /// True while CABLE Input is the Windows default because of BoostDeck. If
    /// this is still true at startup, the last run did not clean up.
    pub routing_active: bool,

    /// The device BoostDeck switched away from, to be restored on exit, on
    /// panic, or on the next launch.
    pub original_default_device_id: Option<String>,

    /// The output picked in the dropdown; `None` plays to whatever the
    /// default was before boost went on.
    pub output_device_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_boost_db: DEFAULT_MAX_BOOST_DB,
            unlock_max_boost: false,
            ring_target_ms: dsp::drift::RING_TARGET_MS,
            routing_active: false,
            original_default_device_id: None,
            output_device_id: None,
        }
    }
}

impl Settings {
    /// The effective gain clamp, honouring the unlock checkbox.
    pub fn max_gain_db(&self) -> f32 {
        if self.unlock_max_boost {
            dsp::GAIN_MAX_DB_UNLOCKED
        } else {
            self.max_boost_db
        }
    }

    /// Reads `path`. A missing file is a first run and gives the defaults; an
    /// unreadable one is logged and also gives the defaults, since nothing
    /// in it is worth refusing to start over.
    pub fn load(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                log::warn!("Could not read {}: {error}; using defaults", path.display());
                return Self::default();
            }
        };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            log::warn!(
                "Could not parse {}: {error}; using defaults",
                path.display()
            );
            Self::default()
        })
    }

    /// Writes `path` atomically: a flushed temp file renamed over the old one.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serialising settings")?;
        let temp = path.with_extension("json.tmp");
        {
            let mut file =
                fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
            file.write_all(json.as_bytes())
                .and_then(|()| file.sync_all())
                .with_context(|| format!("writing {}", temp.display()))?;
        }
        fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
    }
}

/// `settings.json`'s location, or `None` if Windows will not say where the
/// user's app-data folder is.
pub fn path() -> Option<PathBuf> {
    let (qualifier, organisation, application) = presets::PROJECT_DIRS;
    directories::ProjectDirs::from(qualifier, organisation, application)
        .map(|dirs| dirs.config_dir().join(FILE_NAME))
}

/// The one copy of the settings, shared by the UI and the engine's control
/// thread - which records the route through it - so neither saves over the
/// other's changes. The lock is held only to read or change a field and
/// save; the real-time threads never see it.
#[derive(Clone)]
pub struct SharedSettings {
    inner: Arc<Mutex<Settings>>,
    path: Option<PathBuf>,
}

impl SharedSettings {
    /// Loads from `path`, or starts from the defaults when there is none.
    pub fn load(path: Option<PathBuf>) -> Self {
        let settings = path.as_deref().map(Settings::load).unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(settings)),
            path,
        }
    }

    /// A copy of the current settings.
    pub fn get(&self) -> Settings {
        self.lock().clone()
    }

    /// Changes the settings and saves them.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut settings = self.lock();
        change(&mut settings);
        match &self.path {
            Some(path) => settings.save(path),
            None => {
                log::warn!("No app-data folder; settings not saved");
                Ok(())
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Settings> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RoutingJournal for SharedSettings {
    fn saved_original(&self) -> Option<String> {
        let settings = self.lock();
        settings
            .routing_active
            .then(|| settings.original_default_device_id.clone())
            .flatten()
    }

    fn record(&mut self, original: Option<&str>) -> Result<()> {
        self.update(|settings| {
            settings.routing_active = original.is_some();
            settings.original_default_device_id = original.map(str::to_owned);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("boostdeck-settings-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn round_trips_the_routing_record() {
        let path = temp_path("round-trip");
        let settings = Settings {
            routing_active: true,
            original_default_device_id: Some("{0.0.0.00000000}.{speakers}".to_owned()),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file left behind"
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_or_broken_file_gives_the_defaults() {
        let path = temp_path("broken");
        assert_eq!(Settings::load(&path), Settings::default());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// Fields added later must not wipe the routing record of an older file.
    #[test]
    fn missing_fields_take_their_defaults() {
        let path = temp_path("partial");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{ "routing_active": true, "original_default_device_id": "x" }"#,
        )
        .unwrap();
        let settings = Settings::load(&path);
        assert!(settings.routing_active);
        assert_eq!(settings.original_default_device_id.as_deref(), Some("x"));
        assert_eq!(settings.ring_target_ms, dsp::drift::RING_TARGET_MS);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// The engine records the route through the shared settings: on disk at
    /// once, and without losing what the UI changed.
    #[test]
    fn the_journal_records_and_clears_the_route_on_disk() {
        let path = temp_path("journal");
        let mut shared = SharedSettings::load(Some(path.clone()));
        shared
            .update(|s| s.output_device_id = Some("headphones".to_owned()))
            .unwrap();

        shared.record(Some("speakers")).unwrap();
        assert_eq!(shared.saved_original().as_deref(), Some("speakers"));
        let on_disk = Settings::load(&path);
        assert!(on_disk.routing_active);
        assert_eq!(
            on_disk.original_default_device_id.as_deref(),
            Some("speakers")
        );
        assert_eq!(on_disk.output_device_id.as_deref(), Some("headphones"));

        shared.record(None).unwrap();
        assert_eq!(shared.saved_original(), None);
        let on_disk = Settings::load(&path);
        assert!(!on_disk.routing_active);
        assert_eq!(on_disk.output_device_id.as_deref(), Some("headphones"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
