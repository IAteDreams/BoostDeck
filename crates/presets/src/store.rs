//! `PresetStore`: load, save and the operations the UI calls.
//!
//! Step 9. Operations and their rules:
//!
//! | Operation | Rule |
//! | --- | --- |
//! | `create(name, from_current)` | Name trimmed, 1-40 chars, unique ignoring case |
//! | `update(preset)` | Overwrites the active preset ("Save") |
//! | `rename(id, name)` | Same name rules as create |
//! | `duplicate(id)` | New id, name "<name> copy", "<name> copy 2", ... |
//! | `delete(id)` | Confirmed in the UI; the last remaining preset cannot go |
//! | `set_active(id)` | Gain applied through the smoother; missing output device keeps the current one |
//! | `list()` | Sorted by name |
//!
//! Saving is atomic: write `presets.json.tmp`, flush, copy the old file to
//! `presets.json.bak`, then rename the temp over `presets.json`.
//!
//! On load, a parse failure falls back to `.bak`; if that fails too, the
//! broken file is renamed to `presets.broken-<timestamp>.json` and the
//! built-ins are used instead, with a note to the user.
