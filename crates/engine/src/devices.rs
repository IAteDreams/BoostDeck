//! Device enumeration: what endpoints exist, and where VB-Cable is.
//!
//! Coding-order step 2. Everything here runs on the control thread, never on
//! the real-time threads: it allocates, and the COM calls can block.
//!
//! The caller must have initialised COM on its thread first (`initialize_mta`).

use anyhow::{Context as _, Result};
use wasapi::{Device, DeviceEnumerator, DeviceState, Direction, Role};

use crate::{CABLE_INPUT_NAME, CABLE_OUTPUT_NAME};

/// One audio endpoint, flattened to the fields BoostDeck needs.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Endpoint id. This, not the friendly name, is what gets stored in
    /// presets and settings and handed to `IPolicyConfig`: names are not
    /// unique and the user can rename them.
    pub id: String,
    pub name: String,
    pub state: DeviceState,
    /// The endpoint's current mix format, if Windows would report it. Absent
    /// for endpoints that are not active.
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}

impl DeviceInfo {
    fn read(device: &Device) -> Result<Self> {
        let id = device.get_id().context("reading device id")?;
        let name = device
            .get_friendlyname()
            .unwrap_or_else(|_| "<unnamed>".to_owned());
        let state = device
            .get_state()
            .with_context(|| format!("reading state of {name}"))?;

        // Only active endpoints reliably report a format, so a failure here is
        // information rather than an error.
        let format = device.get_device_format().ok();

        Ok(Self {
            id,
            name,
            state,
            sample_rate: format.as_ref().map(|f| f.get_samplespersec()),
            channels: format.as_ref().map(|f| f.get_nchannels()),
        })
    }

    pub fn is_active(&self) -> bool {
        self.state == DeviceState::Active
    }

    /// True if this endpoint is VB-Cable's render side, the one BoostDeck
    /// makes the Windows default. It must never be offered as BoostDeck's own
    /// output: that would feed the cable back into itself.
    pub fn is_cable_input(&self) -> bool {
        self.name.contains(CABLE_INPUT_NAME)
    }

    /// True if this endpoint belongs to any VB-Audio virtual cable, not just
    /// the plain `CABLE Input` this machine routes through. Installs often
    /// carry extra variants (`CABLE In 16ch`, `Cable A`, `Cable B`), and
    /// playing into any of them sends the audio somewhere nobody is
    /// listening, so none may be offered as BoostDeck's output.
    pub fn is_virtual_cable(&self) -> bool {
        self.name.contains("VB-Audio")
    }

    /// True if this endpoint is VB-Cable's capture side, which BoostDeck reads
    /// the mixed system audio from.
    pub fn is_cable_output(&self) -> bool {
        self.name.contains(CABLE_OUTPUT_NAME)
    }

    /// `48000 Hz, 2 ch`, or `format unavailable`.
    pub fn format_label(&self) -> String {
        match (self.sample_rate, self.channels) {
            (Some(rate), Some(ch)) => format!("{rate} Hz, {ch} ch"),
            _ => "format unavailable".to_owned(),
        }
    }
}

/// Every endpoint in one direction, whatever its state.
pub fn list(enumerator: &DeviceEnumerator, direction: Direction) -> Result<Vec<DeviceInfo>> {
    let collection = enumerator
        .get_device_collection(&direction)
        .with_context(|| format!("enumerating {direction} devices"))?;

    let mut devices = Vec::new();
    for device in &collection {
        let device = device.with_context(|| format!("reading a {direction} device"))?;
        devices.push(DeviceInfo::read(&device)?);
    }
    Ok(devices)
}

/// The endpoint Windows currently treats as default for a role.
///
/// There may be none at all — a PC with every output disabled — so this is an
/// `Option` rather than an error.
pub fn default_device(
    enumerator: &DeviceEnumerator,
    direction: Direction,
    role: Role,
) -> Result<Option<DeviceInfo>> {
    match enumerator.get_default_device_for_role(&direction, &role) {
        Ok(device) => Ok(Some(DeviceInfo::read(&device)?)),
        Err(_) => Ok(None),
    }
}

/// One endpoint by id, in any state, or `None` if Windows no longer knows it.
pub fn by_id(enumerator: &DeviceEnumerator, id: &str) -> Result<Option<DeviceInfo>> {
    match enumerator.get_device(id) {
        Ok(device) => Ok(Some(DeviceInfo::read(&device)?)),
        Err(_) => Ok(None),
    }
}

/// The two VB-Cable endpoints, each present or not.
#[derive(Debug, Clone, Default)]
pub struct CableEndpoints {
    /// VB-Cable's render side, `CABLE Input`.
    pub input: Option<DeviceInfo>,
    /// VB-Cable's capture side, `CABLE Output`.
    pub output: Option<DeviceInfo>,
}

impl CableEndpoints {
    /// Both halves found and active. Anything else means the setup screen.
    pub fn is_ready(&self) -> bool {
        matches!((&self.input, &self.output), (Some(i), Some(o)) if i.is_active() && o.is_active())
    }

    /// What to tell the user when the cable is not usable, or `None` when it is.
    pub fn problem(&self) -> Option<String> {
        match (&self.input, &self.output) {
            (None, None) => Some("VB-Cable is not installed".to_owned()),
            (None, Some(_)) => Some(format!("{CABLE_INPUT_NAME} is missing")),
            (Some(_), None) => Some(format!("{CABLE_OUTPUT_NAME} is missing")),
            (Some(input), Some(output)) => {
                let mut inactive = Vec::new();
                if !input.is_active() {
                    inactive.push(format!("{} is {}", input.name, input.state));
                }
                if !output.is_active() {
                    inactive.push(format!("{} is {}", output.name, output.state));
                }
                (!inactive.is_empty()).then(|| inactive.join("; "))
            }
        }
    }

    /// True when both halves run at the same sample rate. A mismatch still
    /// works — Windows resamples — but costs an extra conversion, so the
    /// prerequisites ask the user to match them.
    pub fn rates_match(&self) -> Option<bool> {
        let input = self.input.as_ref()?.sample_rate?;
        let output = self.output.as_ref()?.sample_rate?;
        Some(input == output)
    }
}

/// Locate both VB-Cable endpoints by friendly-name fragment.
pub fn find_cable(enumerator: &DeviceEnumerator) -> Result<CableEndpoints> {
    let render = list(enumerator, Direction::Render)?;
    let capture = list(enumerator, Direction::Capture)?;

    Ok(CableEndpoints {
        input: render.into_iter().find(DeviceInfo::is_cable_input),
        output: capture.into_iter().find(DeviceInfo::is_cable_output),
    })
}

/// The render endpoints BoostDeck may play to: active, and never a virtual
/// cable.
///
/// This is the list behind the output picker, and the pool the control thread
/// falls back through when the current output disappears.
pub fn output_candidates(enumerator: &DeviceEnumerator) -> Result<Vec<DeviceInfo>> {
    Ok(list(enumerator, Direction::Render)?
        .into_iter()
        .filter(|d| d.is_active() && !d.is_virtual_cable())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Friendly names as Windows actually reports them, taken from a machine
    /// with VB-Cable plus the 16-channel variant installed.
    fn device(name: &str) -> DeviceInfo {
        DeviceInfo {
            id: "{0.0.0.00000000}.{test}".to_owned(),
            name: name.to_owned(),
            state: DeviceState::Active,
            sample_rate: Some(48_000),
            channels: Some(2),
        }
    }

    #[test]
    fn recognises_the_cable_endpoints() {
        let input = device("CABLE Input (VB-Audio Virtual Cable)");
        assert!(input.is_cable_input());
        assert!(!input.is_cable_output());

        let output = device("CABLE Output (VB-Audio Virtual Cable)");
        assert!(output.is_cable_output());
        assert!(!output.is_cable_input());
    }

    /// `CABLE In 16ch` is a different cable, so it must not be mistaken for
    /// the endpoint BoostDeck routes the system default to.
    #[test]
    fn extra_cable_variants_are_not_cable_input() {
        for name in [
            "CABLE In 16ch (VB-Audio Virtual Cable)",
            "CABLE Out 16ch (VB-Audio Virtual Cable)",
            "Cable A Input (VB-Audio Cable A)",
        ] {
            assert!(!device(name).is_cable_input(), "{name} matched CABLE Input");
        }
    }

    /// ...but every one of them is still a virtual cable, so none may be
    /// offered as an output device.
    #[test]
    fn every_vb_audio_endpoint_counts_as_a_virtual_cable() {
        for name in [
            "CABLE Input (VB-Audio Virtual Cable)",
            "CABLE In 16ch (VB-Audio Virtual Cable)",
            "Cable A Input (VB-Audio Cable A)",
        ] {
            assert!(device(name).is_virtual_cable(), "{name} slipped through");
        }
    }

    #[test]
    fn real_hardware_is_a_valid_output() {
        for name in [
            "Speakers (Realtek(R) Audio)",
            "ARZOPA (NVIDIA High Definition Audio)",
            "Headphones (2- USB Audio Device)",
        ] {
            let device = device(name);
            assert!(!device.is_virtual_cable(), "{name} was filtered out");
            assert!(!device.is_cable_input());
        }
    }

    #[test]
    fn missing_halves_are_reported() {
        let input = device("CABLE Input (VB-Audio Virtual Cable)");
        let output = device("CABLE Output (VB-Audio Virtual Cable)");

        let ready = CableEndpoints {
            input: Some(input.clone()),
            output: Some(output.clone()),
        };
        assert!(ready.is_ready());
        assert_eq!(ready.problem(), None);
        assert_eq!(ready.rates_match(), Some(true));

        assert_eq!(
            CableEndpoints::default().problem().as_deref(),
            Some("VB-Cable is not installed")
        );

        let half = CableEndpoints {
            input: Some(input),
            output: None,
        };
        assert!(!half.is_ready());
        assert!(half.problem().is_some());
        assert_eq!(half.rates_match(), None);
    }

    #[test]
    fn an_inactive_cable_is_not_ready() {
        let mut output = device("CABLE Output (VB-Audio Virtual Cable)");
        output.state = DeviceState::Disabled;

        let cable = CableEndpoints {
            input: Some(device("CABLE Input (VB-Audio Virtual Cable)")),
            output: Some(output),
        };
        assert!(!cable.is_ready());
        assert!(cable.problem().is_some_and(|p| p.contains("Disabled")));
    }

    #[test]
    fn mismatched_rates_are_flagged() {
        let mut output = device("CABLE Output (VB-Audio Virtual Cable)");
        output.sample_rate = Some(44_100);

        let cable = CableEndpoints {
            input: Some(device("CABLE Input (VB-Audio Virtual Cable)")),
            output: Some(output),
        };
        // Still usable, thanks to autoconvert, just with an extra resample.
        assert!(cable.is_ready());
        assert_eq!(cable.rates_match(), Some(false));
    }
}
