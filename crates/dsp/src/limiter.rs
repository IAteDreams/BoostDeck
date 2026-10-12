//! Peak limiter, always on.
//!
//! Step 4: holds the output at or below `LIMITER_CEILING_DBFS` with an
//! `LIMITER_RELEASE_MS` release, and reports its gain reduction for the meter.
//! Must not allocate.
//!
//! A 2-5 ms look-ahead buffer is a later improvement, not MVP.
