//! Frame-exact rational time (OTIO-style `value/rate`).
//!
//! The model persists times as `f64` seconds for a clean JSON edge, but every
//! boundary that lands in the timeline is snapped to a frame of the source
//! media first, via [`snap_to_frame`]. All arithmetic that decides where a
//! frame boundary lies goes through [`RationalTime`] so float drift can't
//! accumulate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RationalTime {
    /// Count of units.
    pub value: i64,
    /// Units per second (e.g. the frame rate numerator over 1s).
    pub rate: i64,
}

impl RationalTime {
    pub fn new(value: i64, rate: i64) -> Self {
        assert!(rate > 0, "rate must be positive");
        Self { value, rate }
    }

    pub fn from_seconds(seconds: f64, rate: i64) -> Self {
        Self { value: (seconds * rate as f64).round() as i64, rate }
    }

    pub fn to_seconds(self) -> f64 {
        self.value as f64 / self.rate as f64
    }

    pub fn rescaled_to(self, rate: i64) -> Self {
        if self.rate == rate {
            return self;
        }
        Self { value: (self.value as i128 * rate as i128 / self.rate as i128) as i64, rate }
    }
}

#[path = "frame_time.rs"]
mod frame_time;
pub use frame_time::{fps_tick_rate, snap_to_frame, timecode, timecode_frames, to_frame};
pub(crate) use frame_time::{retained_frame_spans, source_frame_range};
