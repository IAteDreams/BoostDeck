//! Schema migrations.
//!
//! Step 9: upgrade a `presets.json` written by an older `schema_version` one
//! step at a time until it matches `SCHEMA_VERSION`, then parse it into the
//! current structs. A version 0 fixture migrating to version 1 is one of the
//! automated tests.
