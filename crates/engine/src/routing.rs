//! Default-device switch and restore.
//!
//! Step 6. Turning boost on, from the control thread:
//!
//! 1. Find VB-Cable's two endpoints. If either is missing, show the setup screen.
//! 2. Read the current default render device id. If it is already CABLE Input
//!    (left over from a crash), use the saved original from settings instead.
//! 3. Save that original id to `settings.json` with `routing_active: true`
//!    **first**, then switch the default to CABLE Input.
//! 4. Start the render thread, then the capture thread.
//!
//! Turning boost off or exiting: stop capture, stop render, restore the
//! original default, clear `routing_active`, save settings.
//!
//! Only the Console and Multimedia roles are touched. The Communications role
//! is left alone so calls in Teams or Discord keep the user's chosen device.
//!
//! Crash safety: restore from a panic hook (`std::panic::set_hook`) and from
//! eframe's `on_exit`. On launch, if `routing_active` is still true the last
//! run did not clean up — restore the saved original right away and say why.
