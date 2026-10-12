//! Control thread: owns devices, routing and stream lifetime.
//!
//! It is the only thread that reacts to device changes: the real-time
//! threads just raise an atomic flag and exit, and Windows device
//! notifications (via `wasapi`'s `DeviceEventCallbacks`) arrive on Windows'
//! own threads, so each callback only posts a message here and returns.
//!
//! | What happened | What this thread does |
//! | --- | --- |
//! | Output device unplugged or disabled | Stop streams, fall back to the saved original (else the first active non-CABLE render device), restart, tell the user |
//! | Output device plugged back in | Offer to switch back; never switch silently mid-playback |
//! | Windows default moved off CABLE Input | Stop streams, do not fight it; offer to re-route, optionally adopting the new device as output |
//! | Sample rate or format changed | Rebuild both streams at the new rate |
//! | Sleep and resume | Same rebuild path |
//! | VB-Cable uninstalled | Restore the original default, stop, show the setup screen |
//!
//! A format change and a sleep both reach this thread the same way: Windows
//! invalidates the streams, a real-time thread fails and sets its error flag,
//! and the poll below rebuilds. Rebuilds back off over `REBUILD_BACKOFF_MS`.
//! If all five attempts fail, restore the original default and report an
//! error rather than leave the user in silence.
//!
//! This module also defines `EngineCommand` and `EngineEvent`, the two message
//! types the UI exchanges with the engine. Gain and limiter values are *not*
//! among them: the UI writes those straight into the `Shared` atomics so
//! slider moves reach the render thread on its next buffer.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use crossbeam_channel::{Receiver, Sender, select};
use rtrb::RingBuffer;
use wasapi::{DeviceEnumerator, DeviceEventCallbacks, DeviceState, Direction, Role};

use crate::devices::{self, DeviceInfo};
use crate::routing::{self, Restored, Route, RoutingJournal};
use crate::rt::{CHANNELS, Shared};
use crate::{REBUILD_BACKOFF_MS, capture, render};

/// How often the control thread checks the real-time threads' error flag
/// and its retry timer when no message arrives.
const POLL: Duration = Duration::from_millis(50);

/// How long streams must run cleanly before the backoff starts again from
/// its first step. Shorter, and a device that fails a few seconds after
/// every start would be retried forever without ever giving up.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

/// What the UI asks of the engine.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    /// Route the Windows default to CABLE Input and start playing.
    Start(StartOptions),
    /// Stop playing and restore the original default output.
    Stop,
    /// Play to this device from now on. Takes effect at once while running.
    SetOutput(String),
    /// Stop, restore, and end the control thread.
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct StartOptions {
    /// The device to play to. `None` means the device the default was
    /// switched away from - what the user was listening to already.
    pub output_id: Option<String>,
    /// Ring target, in milliseconds.
    pub ring_target_ms: u32,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            output_id: None,
            ring_target_ms: dsp::drift::RING_TARGET_MS,
        }
    }
}

/// What the engine tells the UI.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    Status(EngineStatus),
    /// Streams started, on a first start or after a rebuild.
    Playing {
        output: DeviceInfo,
        sample_rate: u32,
    },
    /// The output in use went away; playback moved to `to`.
    OutputFellBack {
        from: String,
        to: DeviceInfo,
    },
    /// The output the user chose is back. Offer to switch; the engine does
    /// not switch by itself mid-playback.
    OutputReturned(DeviceInfo),
    /// Someone made another device the Windows default while boost was on.
    /// Playback has stopped; offer to re-route.
    DefaultMovedOffCable(Option<DeviceInfo>),
    /// The original default output is back.
    Restored(Restored),
    /// Something the user needs to know about, in a sentence.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineStatus {
    Stopped,
    Running,
    /// Waiting to retry. `attempt` counts from 1.
    Reconnecting {
        attempt: usize,
    },
    PausedOutputChanged,
    CableMissing,
}

/// The UI's handle on the engine: the command sender, the event receiver,
/// and the `Shared` atomics for gain and meters. Dropping it shuts the
/// control thread down, which restores the default output.
pub struct Engine {
    commands: Sender<EngineCommand>,
    events: Receiver<EngineEvent>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(journal: Box<dyn RoutingJournal>) -> Result<Self> {
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, events) = crossbeam_channel::unbounded();
        let shared = Arc::new(Shared::new());
        let thread = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("boostdeck-control".to_owned())
                .spawn(move || {
                    if let Err(error) = run(command_rx, event_tx.clone(), shared, journal) {
                        log::error!("Control thread failed: {error:#}");
                        let _ = event_tx.send(EngineEvent::Error(format!("{error:#}")));
                    }
                })
                .context("spawning the control thread")?
        };
        Ok(Self {
            commands,
            events,
            shared,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: EngineCommand) {
        // Fails only once the thread has gone, which `events` reports.
        let _ = self.commands.send(command);
    }

    pub fn events(&self) -> &Receiver<EngineEvent> {
        &self.events
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(EngineCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Posted by the Windows notification callbacks, which may do nothing else.
#[derive(Debug)]
enum Notification {
    StateChanged(String, DeviceState),
    Removed(String),
    Added(String),
    /// The Console default render device changed, to this id if any.
    DefaultChanged(Option<String>),
}

fn run(
    commands: Receiver<EngineCommand>,
    events: Sender<EngineEvent>,
    shared: Arc<Shared>,
    journal: Box<dyn RoutingJournal>,
) -> Result<()> {
    wasapi::initialize_mta()
        .ok()
        .context("initialising COM (MTA)")?;
    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;

    let (notify, notifications) = crossbeam_channel::unbounded();
    let _registration = enumerator
        .register_notification_callback(callbacks(notify))
        .context("registering for device notifications")?;

    let mut control = Control {
        enumerator,
        shared,
        events,
        journal,
        route: None,
        paused_route: None,
        wanted_output: None,
        ring_target_ms: dsp::drift::RING_TARGET_MS,
        streams: None,
        backoff: Backoff::default(),
        retry_at: None,
        status: EngineStatus::Stopped,
    };

    loop {
        select! {
            recv(commands) -> command => match command {
                Ok(EngineCommand::Shutdown) | Err(_) => break,
                Ok(command) => control.handle_command(command),
            },
            recv(notifications) -> notification => {
                if let Ok(notification) = notification {
                    control.handle_notification(notification);
                }
            },
            default(POLL) => {},
        }
        control.poll();
    }

    control.stop();
    Ok(())
}

fn callbacks(notify: Sender<Notification>) -> DeviceEventCallbacks {
    let mut callbacks = DeviceEventCallbacks::new();
    let tx = notify.clone();
    callbacks.set_device_state_callback(move |id, state| {
        let _ = tx.send(Notification::StateChanged(id, state));
    });
    let tx = notify.clone();
    callbacks.set_device_removed_callback(move |id| {
        let _ = tx.send(Notification::Removed(id));
    });
    let tx = notify.clone();
    callbacks.set_device_added_callback(move |id| {
        let _ = tx.send(Notification::Added(id));
    });
    callbacks.set_default_device_callback(move |direction, role, id| {
        // Console and Multimedia move together; one is enough.
        if direction == Direction::Render && role == Role::Console {
            let _ = notify.send(Notification::DefaultChanged(id));
        }
    });
    callbacks
}

struct Control {
    enumerator: DeviceEnumerator,
    shared: Arc<Shared>,
    events: Sender<EngineEvent>,
    journal: Box<dyn RoutingJournal>,
    /// Set while CABLE Input is the default because of us.
    route: Option<Route>,
    /// The route given up when the user moved the default elsewhere, kept
    /// while paused so playback can resume if the default comes back.
    paused_route: Option<Route>,
    /// The output the user asked for; `None` means the original default.
    wanted_output: Option<String>,
    ring_target_ms: u32,
    streams: Option<Streams>,
    backoff: Backoff,
    /// When to try starting streams again, while reconnecting.
    retry_at: Option<Instant>,
    status: EngineStatus,
}

impl Control {
    fn handle_command(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::Start(options) => {
                self.wanted_output = options.output_id;
                self.ring_target_ms = options.ring_target_ms;
                self.start();
            }
            EngineCommand::Stop => self.stop(),
            EngineCommand::SetOutput(id) => {
                self.wanted_output = Some(id);
                if self.streams.is_some() {
                    self.stop_streams();
                    self.backoff.reset();
                    self.try_start_streams();
                }
            }
            EngineCommand::Shutdown => unreachable!("handled by the loop"),
        }
    }

    fn start(&mut self) {
        self.stop_streams();
        self.paused_route = None;
        self.retry_at = None;
        self.backoff.reset();

        if self.route.is_none() {
            let saved = self.journal.saved_original();
            let journal = &mut self.journal;
            match routing::route_to_cable(&self.enumerator, saved.as_deref(), |id| {
                journal.record(Some(id))
            }) {
                Ok(route) => self.route = Some(route),
                Err(error) => {
                    log::error!("Routing failed: {error:#}");
                    let cable_missing = devices::find_cable(&self.enumerator)
                        .map(|cable| cable.problem().is_some())
                        .unwrap_or(false);
                    self.set_status(if cable_missing {
                        EngineStatus::CableMissing
                    } else {
                        EngineStatus::Stopped
                    });
                    self.send(EngineEvent::Error(format!(
                        "Could not switch the default output to VB-Cable: {error:#}"
                    )));
                    return;
                }
            }
        }
        self.try_start_streams();
    }

    /// Stops playing and puts the original default back.
    fn stop(&mut self) {
        self.stop_streams();
        self.paused_route = None;
        self.retry_at = None;
        self.end_routing();
        if self.status != EngineStatus::CableMissing {
            self.set_status(EngineStatus::Stopped);
        }
    }

    fn end_routing(&mut self) {
        let Some(route) = self.route.take() else {
            return;
        };
        match routing::restore(&self.enumerator, Some(&route.original.id)) {
            Ok(restored) => {
                if let Err(error) = self.journal.record(None) {
                    log::error!("Clearing the routing record failed: {error:#}");
                }
                self.send(EngineEvent::Restored(restored));
            }
            Err(error) => {
                // The record stays, so the next launch tries again.
                log::error!("Restoring the default output failed: {error:#}");
                self.send(EngineEvent::Error(format!(
                    "Could not restore your default output: {error:#}. \
                     Set your speakers as the default in Sound settings."
                )));
            }
        }
    }

    /// Starts a stream pair on the right output; on failure, schedules the
    /// next attempt.
    fn try_start_streams(&mut self) {
        match self.start_streams() {
            Ok(()) => {
                self.retry_at = None;
                self.set_status(EngineStatus::Running);
            }
            Err(error) => {
                log::warn!("Starting the streams failed: {error:#}");
                self.schedule_rebuild();
            }
        }
    }

    fn start_streams(&mut self) -> Result<()> {
        let route = self.route.as_ref().context("not routed")?;
        let cable = devices::find_cable(&self.enumerator)?;
        if let Some(problem) = cable.problem() {
            bail!("VB-Cable is not ready: {problem}");
        }
        let cable_output = cable.output.context("CABLE Output went missing")?;

        let candidates = devices::output_candidates(&self.enumerator)?;
        let wanted = self.wanted_output.as_deref().unwrap_or(&route.original.id);
        let output = choose_output(wanted, &route.original.id, &candidates)
            .context("no real output device to play to")?
            .clone();
        if output.id != wanted {
            log::warn!("Output {wanted} is unavailable; playing to {}", output.name);
            self.send(EngineEvent::OutputFellBack {
                from: wanted.to_owned(),
                to: output.clone(),
            });
        }

        let sample_rate = capture::device_sample_rate(&self.enumerator, &output.id)?;
        self.streams = Some(Streams::start(
            &self.shared,
            &cable_output.id,
            &output,
            sample_rate,
            self.ring_target_ms,
        )?);
        log::info!("Playing to {} at {sample_rate} Hz", output.name);
        self.send(EngineEvent::Playing {
            output,
            sample_rate,
        });
        Ok(())
    }

    fn stop_streams(&mut self) {
        if let Some(streams) = self.streams.take() {
            streams.stop(&self.shared);
        }
    }

    /// After a failure: wait the next backoff step, or give up and restore.
    fn schedule_rebuild(&mut self) {
        match self.backoff.next_delay() {
            Some(delay) => {
                self.retry_at = Some(Instant::now() + delay);
                self.set_status(EngineStatus::Reconnecting {
                    attempt: self.backoff.attempts(),
                });
            }
            None => {
                log::error!("Giving up after {} attempts", REBUILD_BACKOFF_MS.len());
                self.stop();
                self.send(EngineEvent::Error(format!(
                    "Lost the audio device and could not reconnect after {} attempts. \
                     Your original output has been restored.",
                    REBUILD_BACKOFF_MS.len()
                )));
            }
        }
    }

    /// Between messages: notice a failed real-time thread, run a due retry,
    /// and forgive old failures once playback has been healthy a while.
    fn poll(&mut self) {
        if let Some(streams) = &self.streams {
            let error = self.shared.error();
            if error != crate::rt::RtError::None {
                log::warn!("An audio thread stopped: {error:?}; rebuilding");
                self.stop_streams();
                self.schedule_rebuild();
            } else if streams.started.elapsed() >= HEALTHY_AFTER {
                self.backoff.reset();
            }
        }
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            self.try_start_streams();
        }
    }

    fn handle_notification(&mut self, notification: Notification) {
        log::debug!("Device notification: {notification:?}");
        match notification {
            Notification::DefaultChanged(id) => self.default_changed(id),
            Notification::StateChanged(id, DeviceState::Active) | Notification::Added(id) => {
                self.device_arrived(&id)
            }
            Notification::StateChanged(id, _) | Notification::Removed(id) => self.device_left(&id),
        }
    }

    /// Someone else may have changed the default. Our own switches cause
    /// this too, and notifications can arrive late, so the question is
    /// settled by asking Windows what the default is now.
    fn default_changed(&mut self, _id: Option<String>) {
        let current = devices::default_device(&self.enumerator, Direction::Render, Role::Console)
            .ok()
            .flatten();
        let Some(route) = &self.route else {
            self.default_changed_while_paused(current.as_ref());
            return;
        };
        if current
            .as_ref()
            .is_some_and(|d| d.id == route.cable_input.id)
        {
            return;
        }
        log::warn!(
            "The default output moved to {:?}; pausing",
            current.as_ref().map(|d| &d.name)
        );
        self.stop_streams();
        self.retry_at = None;
        // Not ours to undo: the user picked a new default. Drop the record so
        // the next launch does not "restore" over their choice. Keep the route
        // itself, in case Windows hands the default back to the cable.
        self.paused_route = self.route.take();
        routing::arm_panic_restore(None);
        if let Err(error) = self.journal.record(None) {
            log::error!("Clearing the routing record failed: {error:#}");
        }
        self.set_status(EngineStatus::PausedOutputChanged);
        self.send(EngineEvent::DefaultMovedOffCable(current));
    }

    /// Paused because the default moved, and now it is CABLE Input again.
    /// Nothing reads the cable while paused, so the user would hear nothing.
    /// That happens when the device they moved to goes away - Bluetooth
    /// headphones switched off - and Windows picks the cable as the next
    /// default; or when they pick the cable by hand to get boost back.
    /// Either way the answer is to play again, on the same route.
    fn default_changed_while_paused(&mut self, current: Option<&DeviceInfo>) {
        let Some(paused) = &self.paused_route else {
            return;
        };
        if self.status != EngineStatus::PausedOutputChanged
            || !current.is_some_and(|d| d.id == paused.cable_input.id)
        {
            return;
        }
        log::warn!("The default output is CABLE Input again; resuming");
        if let Err(error) = self.journal.record(Some(&paused.original.id)) {
            log::error!("Saving the routing record failed: {error:#}");
        }
        self.route = self.paused_route.take();
        routing::arm_panic_restore(self.route.as_ref().map(|r| r.original.id.as_str()));
        self.backoff.reset();
        self.try_start_streams();
    }

    fn device_left(&mut self, id: &str) {
        let Some(route) = &self.route else {
            return;
        };
        let is_cable = id == route.cable_input.id
            || devices::by_id(&self.enumerator, id)
                .ok()
                .flatten()
                .is_some_and(|d| d.is_cable_output());
        if is_cable {
            log::error!("VB-Cable went away; restoring the default output");
            self.stop_streams();
            self.retry_at = None;
            self.end_routing();
            self.set_status(EngineStatus::CableMissing);
            self.send(EngineEvent::Error(
                "VB-Cable is no longer available. Your original output has been restored."
                    .to_owned(),
            ));
            return;
        }
        let playing_to = self.streams.as_ref().map(|s| s.output.id.as_str());
        if playing_to == Some(id) {
            // The stream would fail on its own, but only after the event
            // timeout. Move to the fallback now.
            log::warn!("The output device went away; falling back");
            self.stop_streams();
            self.backoff.reset();
            self.try_start_streams();
        }
    }

    fn device_arrived(&mut self, id: &str) {
        let Some(route) = &self.route else {
            return;
        };
        let wanted = self.wanted_output.as_deref().unwrap_or(&route.original.id);
        let playing_to = self.streams.as_ref().map(|s| s.output.id.as_str());
        if id != wanted || playing_to == Some(wanted) {
            return;
        }
        if let Ok(Some(device)) = devices::by_id(&self.enumerator, id)
            && device.is_active()
        {
            if self.streams.is_none() && self.retry_at.is_some() {
                // Nothing is playing anyway: no reason to wait for the timer.
                self.retry_at = Some(Instant::now());
            } else {
                self.send(EngineEvent::OutputReturned(device));
            }
        }
    }

    fn set_status(&mut self, status: EngineStatus) {
        if self.status != status {
            self.status = status;
            self.send(EngineEvent::Status(status));
        }
    }

    fn send(&self, event: EngineEvent) {
        let _ = self.events.send(event);
    }
}

/// A running capture and render thread pair.
struct Streams {
    render: JoinHandle<Result<()>>,
    capture: JoinHandle<Result<()>>,
    output: DeviceInfo,
    started: Instant,
}

impl Streams {
    fn start(
        shared: &Arc<Shared>,
        cable_output_id: &str,
        output: &DeviceInfo,
        sample_rate: u32,
        ring_target_ms: u32,
    ) -> Result<Self> {
        let target_fill_frames = sample_rate * ring_target_ms / 1_000;
        let ring_capacity =
            (sample_rate as f32 * dsp::drift::RING_CAPACITY_SECS) as usize * CHANNELS as usize;
        // A fresh ring every time: priming expects to start from empty.
        let (producer, consumer) = RingBuffer::<f32>::new(ring_capacity);
        shared.clear_stop();
        shared.clear_error();
        shared.reset_counters();

        // Render first, so the device is ready the moment capture produces.
        let render = {
            let shared = Arc::clone(shared);
            let config = render::RenderConfig {
                device_id: output.id.clone(),
                sample_rate,
                target_fill_frames,
            };
            thread::Builder::new()
                .name("boostdeck-render".to_owned())
                .spawn(move || render::run(config, consumer, shared))
                .context("spawning the render thread")?
        };
        let capture = {
            let thread_shared = Arc::clone(shared);
            let config = capture::CaptureConfig {
                device_id: cable_output_id.to_owned(),
                sample_rate,
            };
            match thread::Builder::new()
                .name("boostdeck-capture".to_owned())
                .spawn(move || capture::run(config, producer, thread_shared))
            {
                Ok(capture) => capture,
                Err(error) => {
                    // Never leave a render thread running with no one to stop it.
                    shared.request_stop();
                    let _ = render.join();
                    return Err(anyhow!(error).context("spawning the capture thread"));
                }
            }
        };
        Ok(Self {
            render,
            capture,
            output: output.clone(),
            started: Instant::now(),
        })
    }

    fn stop(self, shared: &Shared) {
        shared.request_stop();
        for (name, thread) in [("capture", self.capture), ("render", self.render)] {
            match thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("The {name} thread ended with: {error:#}"),
                Err(_) => log::error!("The {name} thread panicked"),
            }
        }
    }
}

/// The device to play to: the one asked for if it is usable, else the
/// original default, else the first real output.
fn choose_output<'a>(
    wanted: &str,
    original: &str,
    candidates: &'a [DeviceInfo],
) -> Option<&'a DeviceInfo> {
    candidates
        .iter()
        .find(|d| d.id == wanted)
        .or_else(|| candidates.iter().find(|d| d.id == original))
        .or_else(|| candidates.first())
}

/// Counts rebuild attempts through `REBUILD_BACKOFF_MS`.
#[derive(Debug, Default)]
struct Backoff {
    attempts: usize,
}

impl Backoff {
    /// The wait before the next attempt, or `None` once every step is used.
    fn next_delay(&mut self) -> Option<Duration> {
        let delay = REBUILD_BACKOFF_MS.get(self.attempts)?;
        self.attempts += 1;
        Some(Duration::from_millis(*delay))
    }

    fn attempts(&self) -> usize {
        self.attempts
    }

    fn reset(&mut self) {
        self.attempts = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            state: DeviceState::Active,
            sample_rate: Some(48_000),
            channels: Some(2),
        }
    }

    #[test]
    fn backoff_walks_the_schedule_then_gives_up() {
        let mut backoff = Backoff::default();
        let delays: Vec<u64> = std::iter::from_fn(|| backoff.next_delay())
            .map(|d| d.as_millis() as u64)
            .collect();
        assert_eq!(delays, REBUILD_BACKOFF_MS);
        assert_eq!(backoff.attempts(), 5);
        assert_eq!(backoff.next_delay(), None);

        backoff.reset();
        assert_eq!(backoff.next_delay(), Some(Duration::from_millis(250)));
    }

    #[test]
    fn plays_to_the_wanted_output_when_it_is_there() {
        let candidates = [device("speakers"), device("headphones")];
        let output = choose_output("headphones", "speakers", &candidates).unwrap();
        assert_eq!(output.id, "headphones");
    }

    /// Headphones unplugged: back to the original default, as the README's
    /// table says.
    #[test]
    fn falls_back_to_the_original_when_the_wanted_output_is_gone() {
        let candidates = [device("monitor"), device("speakers")];
        let output = choose_output("headphones", "speakers", &candidates).unwrap();
        assert_eq!(output.id, "speakers");
    }

    #[test]
    fn falls_back_to_the_first_real_output_when_both_are_gone() {
        let candidates = [device("monitor"), device("speakers")];
        let output = choose_output("headphones", "usb", &candidates).unwrap();
        assert_eq!(output.id, "monitor");
    }

    #[test]
    fn no_real_output_means_nothing_to_play_to() {
        assert!(choose_output("headphones", "speakers", &[]).is_none());
    }
}
