//! App settings, stored next to `presets.json` under `%APPDATA%`.
//!
//! Step 6 adds load and save; `routing_active` and `original_default_device_id`
//! are what let the next launch notice that the last run did not clean up and
//! restore the user's device.

use serde::{Deserialize, Serialize};

/// Highest gain the slider offers unless the user unlocks it.
const DEFAULT_MAX_BOOST_DB: f32 = dsp::GAIN_MAX_DB;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_boost_db: DEFAULT_MAX_BOOST_DB,
            unlock_max_boost: false,
            ring_target_ms: dsp::drift::RING_TARGET_MS,
            routing_active: false,
            original_default_device_id: None,
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
}
