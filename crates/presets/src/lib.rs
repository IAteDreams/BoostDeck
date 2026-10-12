//! Preset data model and versioned JSON storage.
//!
//! A preset is a named snapshot of gain and output device. All presets live
//! in one versioned `presets.json` under `%APPDATA%`, next to the app's
//! `settings.json`.

pub mod migrate;
pub mod model;
pub mod store;

pub use model::{DeviceRef, Preset, PresetFile};
pub use store::{LoadOutcome, PresetError, PresetStore};

/// Qualifier, organisation and application passed to
/// `directories::ProjectDirs`, which resolves `presets.json`'s folder.
pub const PROJECT_DIRS: (&str, &str, &str) = ("com", "BoostDeck", "BoostDeck");

/// Schema version written to new preset files. `migrate` upgrades older files
/// step by step before they are parsed into the current structs.
pub const SCHEMA_VERSION: u32 = 1;

/// Preset names are trimmed and must fall within this length, and be unique
/// ignoring case.
pub const NAME_LEN_RANGE: (usize, usize) = (1, 40);
