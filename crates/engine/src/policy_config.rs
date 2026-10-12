//! `IPolicyConfig`, declared by hand.
//!
//! Windows has no public API for changing the default audio device. Every
//! switcher tool uses this undocumented COM interface, which has stayed stable
//! across many Windows versions.
//!
//! Step 6: declare the interface here against `windows` 0.62 — method order
//! must match exactly. The `com-policy-config` crate wraps it but depends on
//! `windows` 0.61, whose types do not mix with ours; its source is the
//! reference for the layout and GUIDs.
