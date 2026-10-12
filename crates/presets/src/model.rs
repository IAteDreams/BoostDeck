//! Preset and file structs.
//!
//! The MVP fields only: id, name, gain, output device. Later features add
//! fields with `#[serde(default)]` so old files keep parsing. The limiter has
//! no settings here: it is always on, at a fixed ceiling, in the MVP.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::SCHEMA_VERSION;

/// A named snapshot of the boost and where it plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: Uuid,
    pub name: String,
    /// Clamped to the app's gain range when it is applied, not here: the
    /// range depends on the unlock setting, which is the app's.
    pub gain_db: f32,
    /// The output to play to. `None` keeps whatever output is in use.
    #[serde(default)]
    pub output_device: Option<DeviceRef>,
}

/// An output device as a preset remembers it. Matched by id first; the name
/// is the fallback for a device Windows gave a new id, and what the UI shows
/// when the device is not connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
}

/// `presets.json` as written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    pub schema_version: u32,
    /// The preset last loaded, if it still exists.
    #[serde(default)]
    pub active: Option<Uuid>,
    pub presets: Vec<Preset>,
}

impl PresetFile {
    /// What a first run starts with: Default (0 dB), Quiet videos (+6 dB)
    /// and Max (+12 dB), none tied to an output, with Default active.
    pub fn built_ins() -> Self {
        let presets: Vec<Preset> = [("Default", 0.0), ("Quiet videos", 6.0), ("Max", 12.0)]
            .into_iter()
            .map(|(name, gain_db)| Preset {
                id: Uuid::new_v4(),
                name: name.to_owned(),
                gain_db,
                output_device: None,
            })
            .collect();
        Self {
            schema_version: SCHEMA_VERSION,
            active: presets.first().map(|p| p.id),
            presets,
        }
    }
}
