//! Soft clipper.
//!
//! Step 4: rounds the signal off above `SOFT_CLIP_KNEE` of the ceiling so the
//! limiter has less work to do and hard edges never reach the output. Must not
//! allocate.
