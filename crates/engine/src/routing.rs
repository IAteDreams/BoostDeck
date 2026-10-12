//! Default-device switch and restore.
//!
//! Turning boost on, from the control thread ([`route_to_cable`]):
//!
//! 1. Find VB-Cable's two endpoints. If either is missing, show the setup screen.
//! 2. Read the current default render device id. If it is already CABLE Input
//!    (left over from a crash), use the saved original from settings instead.
//! 3. Save that original id to `settings.json` with `routing_active: true`
//!    **first**, then switch the default to CABLE Input.
//! 4. Start the render thread, then the capture thread.
//!
//! Turning boost off or exiting ([`restore`]): stop capture, stop render,
//! restore the original default, clear `routing_active`, save settings.
//!
//! Only the Console and Multimedia roles are touched. The Communications role
//! is left alone so calls in Teams or Discord keep the user's chosen device.
//!
//! Crash safety: restore from a panic hook ([`install_panic_hook`]) and from
//! eframe's `on_exit`. On launch, if `routing_active` is still true the last
//! run did not clean up — restore the saved original right away and say why.
//! A kill from Task Manager runs neither hook, which is what the launch check
//! is for.
//!
//! Settings belong to the app, so the engine never touches `settings.json`:
//! the caller passes in the saved original and a closure that records the new
//! one.

use std::sync::Mutex;

use anyhow::{Context as _, Result, bail};
use wasapi::{DeviceEnumerator, Direction, Role};
use windows::Win32::Media::Audio::{ERole, eConsole, eMultimedia};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

use crate::devices::{self, DeviceInfo};
use crate::policy_config;

/// The roles BoostDeck switches. Not Communications.
const ROLES: [ERole; 2] = [eConsole, eMultimedia];

/// The device to put back if the process panics while routed. Set by
/// [`route_to_cable`], cleared by [`restore`].
static RESTORE_ON_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// Where the routing record lives: `routing_active` and
/// `original_default_device_id` in the app's `settings.json`. The control
/// thread owns one and calls it around every switch, so the record is always
/// written before the default changes and cleared after it changes back.
pub trait RoutingJournal: Send {
    /// The original an earlier run recorded and never cleared, if any.
    fn saved_original(&self) -> Option<String>;
    /// `Some(id)`: a route to undo, saved with `routing_active: true`.
    /// `None`: nothing to undo any more.
    fn record(&mut self, original: Option<&str>) -> Result<()>;
}

/// Where the default output went when boost was turned on.
#[derive(Debug, Clone)]
pub struct Route {
    /// The device to restore on the way out.
    pub original: DeviceInfo,
    /// CABLE Input, now the default.
    pub cable_input: DeviceInfo,
}

/// Where the default output went when it was restored.
#[derive(Debug, Clone)]
pub struct Restored {
    pub device: DeviceInfo,
    /// True when the saved original was gone or unusable and another real
    /// output was chosen instead. Worth telling the user.
    pub fell_back: bool,
}

/// Makes CABLE Input the Windows default output, for the Console and
/// Multimedia roles.
///
/// `saved_original` is the original recorded by an earlier run, if
/// `routing_active` was left set. `record` is called with the original's id
/// before anything is switched, and must persist it with `routing_active:
/// true`; if it fails, nothing is switched.
pub fn route_to_cable(
    enumerator: &DeviceEnumerator,
    saved_original: Option<&str>,
    record: impl FnOnce(&str) -> Result<()>,
) -> Result<Route> {
    let cable = devices::find_cable(enumerator)?;
    if let Some(problem) = cable.problem() {
        bail!("VB-Cable is not ready: {problem}");
    }
    let cable_input = cable.input.context("CABLE Input went missing")?;

    let current = devices::default_device(enumerator, Direction::Render, Role::Console)?;
    let candidates = devices::output_candidates(enumerator)?;
    let original = choose_original(current.as_ref(), saved_original, &candidates)
        .context("no real output device to restore to afterwards")?;

    record(&original.id).context("saving the original device before switching")?;
    *lock_restore_on_panic() = Some(original.id.clone());

    set_default_output(&cable_input.id)?;
    log::info!(
        "Default output switched from {} to {}",
        original.name,
        cable_input.name
    );
    Ok(Route {
        original,
        cable_input,
    })
}

/// Puts the original default output back, or the first real output if the
/// original is gone. The caller then clears `routing_active`.
pub fn restore(enumerator: &DeviceEnumerator, original_id: Option<&str>) -> Result<Restored> {
    let candidates = devices::output_candidates(enumerator)?;
    let restored = choose_restore_target(original_id, &candidates)
        .context("no real output device to restore the default to")?;

    set_default_output(&restored.device.id)?;
    lock_restore_on_panic().take();
    if restored.fell_back {
        log::warn!(
            "The original output {original_id:?} is gone; default restored to {}",
            restored.device.name
        );
    } else {
        log::info!("Default output restored to {}", restored.device.name);
    }
    Ok(restored)
}

/// [`restore`], run on a short-lived thread of its own with COM in the
/// multithreaded apartment, for callers on a thread whose apartment is
/// someone else's: eframe's main thread belongs to winit, which wants it
/// single-threaded for drag and drop.
pub fn restore_on_own_thread(original_id: Option<String>) -> Result<Restored> {
    std::thread::Builder::new()
        .name("boostdeck-restore".to_owned())
        .spawn(move || {
            wasapi::initialize_mta()
                .ok()
                .context("initialising COM (MTA)")?;
            let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;
            restore(&enumerator, original_id.as_deref())
        })
        .context("spawning the restore thread")?
        .join()
        .map_err(|_| anyhow::anyhow!("the restore thread panicked"))?
}

/// Sets the default render device for the Console and Multimedia roles.
pub fn set_default_output(device_id: &str) -> Result<()> {
    for role in ROLES {
        policy_config::set_default_endpoint(device_id, role)?;
    }
    Ok(())
}

/// Chains a panic hook that restores the original default output if the
/// process panics while routed. Call once, at startup.
///
/// `routing_active` stays set in the settings, so the next launch still says
/// the last run ended badly. A process killed outright runs no hook at all;
/// the launch check covers that.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // `try_lock`: if the panicking thread holds the lock, waiting would
        // hang the panic. That lock is only ever held for an assignment.
        let original = RESTORE_ON_PANIC
            .try_lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(original) = original {
            // The panicking thread may not have COM yet. A thread already in
            // the other apartment refuses the call but can still use COM, so
            // the result does not matter.
            // SAFETY: plain COM initialisation of the current thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            match set_default_output(&original) {
                Ok(()) => eprintln!("BoostDeck panicked; default output restored."),
                Err(error) => eprintln!(
                    "BoostDeck panicked and could not restore the default output: {error:#}"
                ),
            }
        }
        previous(info);
    }));
}

fn lock_restore_on_panic() -> std::sync::MutexGuard<'static, Option<String>> {
    RESTORE_ON_PANIC
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The device to restore when boost is turned off.
///
/// The current default, unless it is CABLE Input: then an earlier run left it
/// there, and the saved original is the right answer. Failing that, the
/// first real output, so the user is never restored into the cable.
fn choose_original(
    current: Option<&DeviceInfo>,
    saved: Option<&str>,
    candidates: &[DeviceInfo],
) -> Option<DeviceInfo> {
    if let Some(current) = current.filter(|d| !d.is_cable_input()) {
        return Some(current.clone());
    }
    saved
        .and_then(|id| candidates.iter().find(|d| d.id == id))
        .or_else(|| candidates.first())
        .cloned()
}

/// The saved original if it is still a usable real output, otherwise the
/// first one that is.
fn choose_restore_target(original: Option<&str>, candidates: &[DeviceInfo]) -> Option<Restored> {
    if let Some(device) = original.and_then(|id| candidates.iter().find(|d| d.id == id)) {
        return Some(Restored {
            device: device.clone(),
            fell_back: false,
        });
    }
    candidates.first().map(|device| Restored {
        device: device.clone(),
        fell_back: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasapi::DeviceState;

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            state: DeviceState::Active,
            sample_rate: Some(48_000),
            channels: Some(2),
        }
    }

    fn speakers() -> DeviceInfo {
        device("speakers", "Speakers (Realtek(R) Audio)")
    }

    fn monitor() -> DeviceInfo {
        device("monitor", "ARZOPA (NVIDIA High Definition Audio)")
    }

    fn cable() -> DeviceInfo {
        device("cable", "CABLE Input (VB-Audio Virtual Cable)")
    }

    /// What `output_candidates` would return: real outputs only.
    fn candidates() -> Vec<DeviceInfo> {
        vec![speakers(), monitor()]
    }

    #[test]
    fn the_current_default_is_the_original() {
        let original = choose_original(Some(&monitor()), Some("speakers"), &candidates());
        assert_eq!(original.unwrap().id, "monitor");
    }

    /// Already on CABLE Input means an earlier run did not clean up: the
    /// device it saved is the one to go back to, not the cable.
    #[test]
    fn a_leftover_cable_default_uses_the_saved_original() {
        let original = choose_original(Some(&cable()), Some("monitor"), &candidates());
        assert_eq!(original.unwrap().id, "monitor");
    }

    /// Cable as default with nothing saved - set by hand - or with a saved
    /// device that has since gone: fall back to a real output.
    #[test]
    fn a_cable_default_with_no_usable_saved_original_falls_back() {
        for saved in [None, Some("unplugged")] {
            let original = choose_original(Some(&cable()), saved, &candidates());
            assert_eq!(original.unwrap().id, "speakers", "saved {saved:?}");
        }
    }

    #[test]
    fn no_default_at_all_falls_back_to_a_real_output() {
        let original = choose_original(None, None, &candidates());
        assert_eq!(original.unwrap().id, "speakers");
    }

    #[test]
    fn no_real_output_means_no_original() {
        assert!(choose_original(Some(&cable()), None, &[]).is_none());
    }

    #[test]
    fn restores_the_original_when_it_is_still_there() {
        let restored = choose_restore_target(Some("monitor"), &candidates()).unwrap();
        assert_eq!(restored.device.id, "monitor");
        assert!(!restored.fell_back);
    }

    /// Unplugged, disabled, or a cable id that should never have been saved:
    /// none of them are candidates, so the user still gets real speakers.
    #[test]
    fn restores_a_real_output_when_the_original_is_gone() {
        for original in [Some("unplugged"), Some("cable"), None] {
            let restored = choose_restore_target(original, &candidates()).unwrap();
            assert_eq!(restored.device.id, "speakers", "original {original:?}");
            assert!(restored.fell_back);
        }
    }
}
