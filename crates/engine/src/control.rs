//! Control thread: owns devices, routing and stream lifetime.
//!
//! Step 7. It is the only thread that reacts to device changes: the real-time
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
//! Rebuilds back off over `REBUILD_BACKOFF_MS`. If all five attempts fail,
//! restore the original default and report an error.
//!
//! This module also defines `EngineCommand` and `EngineEvent`, the two message
//! types the UI exchanges with the engine. Gain and limiter values are *not*
//! among them: the UI writes those straight into the `Shared` atomics so
//! slider moves reach the render thread on its next buffer.
