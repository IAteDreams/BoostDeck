//! Ctrl+C in a console window.
//!
//! Windows ends a console process on Ctrl+C without running any of its
//! cleanup: no `Drop`, no eframe `on_exit`, so the cable would stay the
//! default. Catching it turns Ctrl+C into a flag the program polls and then
//! leaves through its normal path, restoring the default output on the way.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows_core::BOOL;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Catches Ctrl+C (and Ctrl+Break), which from now on only sets the flag
/// [`interrupted`] reads. Harmless in a process with no console.
pub fn catch_ctrl_c() -> Result<()> {
    // SAFETY: registers a plain function that only stores to an atomic.
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl_c), true) }.context("installing the Ctrl+C handler")
}

/// True once Ctrl+C has been pressed.
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

unsafe extern "system" fn on_ctrl_c(_ctrl_type: u32) -> BOOL {
    INTERRUPTED.store(true, Ordering::Relaxed);
    // Handled: Windows must not end the process before it restores.
    true.into()
}
