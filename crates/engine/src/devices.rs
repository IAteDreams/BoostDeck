//! Device enumeration.
//!
//! Step 2: list active render and capture endpoints with their friendly names
//! and ids, and find the two VB-Cable endpoints by name fragment
//! (`CABLE_INPUT_NAME`, `CABLE_OUTPUT_NAME`). If either is missing the app
//! shows the setup screen.
//!
//! CABLE Input must never be offered as BoostDeck's own output device; that
//! would feed the cable back into itself.
