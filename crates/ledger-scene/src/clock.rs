//! Fixed clock and seed for every scene.
//!
//! Wall time is not a scene input. Two replays share these constants.

use chrono::{DateTime, Duration, Utc};

/// Seed mixed into the scene signing key. The scripts do not draw other randomness.
pub const SCENE_SEED: u64 = 35;

/// Monday 2020-01-06 15:00:00Z. Outside a live six-hour poll window.
pub fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(1_578_322_800, 0).expect("scene epoch is a real timestamp")
}

/// `epoch + seconds`.
#[must_use]
pub fn at(seconds: i64) -> DateTime<Utc> {
    epoch() + Duration::seconds(seconds)
}

/// Nanoseconds on the scene clock.
#[must_use]
pub fn ns(time: DateTime<Utc>) -> i128 {
    i128::from(time.timestamp()).saturating_mul(1_000_000_000)
        + i128::from(time.timestamp_subsec_nanos())
}
