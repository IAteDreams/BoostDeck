//! Peak meters.
//!
//! Step 4: tracks pre-limiter and post-limiter peaks per buffer. The render
//! thread stores these into the `Shared` atomics; the UI reads and resets
//! them. Must not allocate.
