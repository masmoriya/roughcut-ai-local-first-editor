/// Express an fps that may be fractional (29.97 = 30000/1001) as a tick rate
/// usable for exact frame indexing: ticks = round(fps * 1001).
pub fn fps_tick_rate(fps: f64) -> i64 {
    ((fps * 1001.0).round() as i64).max(1)
}

/// Snap a time in seconds to the nearest frame boundary of media at `fps`.
pub fn snap_to_frame(seconds: f64, fps: f64) -> f64 {
    if fps <= 0.0 {
        return seconds;
    }
    let frame = (seconds * fps).round();
    frame / fps
}

/// Convert seconds to a whole frame index at `fps` (nearest).
pub fn to_frame(seconds: f64, fps: f64) -> i64 {
    (seconds * fps).round() as i64
}

pub(crate) fn source_frame_range(start: f64, end: f64, duration: f64, fps: f64) -> (i64, i64) {
    let max = to_frame(duration, fps).max(0);
    (to_frame(start, fps).clamp(0, max), to_frame(end, fps).clamp(0, max))
}

pub(crate) fn retained_frame_spans(
    source_ranges: impl IntoIterator<Item = (f64, f64)>, duration: f64, fps: f64,
) -> impl Iterator<Item = (i64, i64, i64, i64)> {
    let mut timeline_end = 0;
    source_ranges.into_iter().filter_map(move |(start, end)| {
        let (source_in, source_out) = source_frame_range(start, end, duration, fps);
        if source_out <= source_in { return None; }
        let timeline_in = timeline_end;
        timeline_end += source_out - source_in;
        Some((source_in, source_out, timeline_in, timeline_end))
    })
}

/// SMPTE-ish timecode (non-drop) from a whole frame count. EDL math must be
/// done in frames end-to-end — rounding endpoints independently can make
/// source and record durations differ by a frame, which NLE importers reject.
pub fn timecode_frames(total_frames: i64, fps_i: i64) -> String {
    let fps_i = fps_i.max(1);
    let f = total_frames % fps_i;
    let total_secs = total_frames / fps_i;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = total_secs / 3600;
    format!("{h:02}:{m:02}:{s:02}:{f:02}")
}

/// SMPTE-ish timecode (non-drop) from seconds.
pub fn timecode(seconds: f64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as i64;
    timecode_frames(to_frame(seconds, fps), fps_i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_is_frame_exact() {
        let s = snap_to_frame(1.004, 30.0);
        assert!((s - 1.0).abs() < 1e-9);
        assert_eq!(to_frame(s, 30.0), 30);
    }

    #[test]
    fn timecode_formats() {
        assert_eq!(timecode(3661.5, 30.0), "01:01:01:15");
    }
    #[test]
    fn btn_fractional_rate_stays_on_source_frames() {
        let fps = 30_000.0 / 1_001.0;
        let duration = 3172.135633;
        let frames = to_frame(duration, fps);
        assert_eq!(frames, 95069);
        assert_eq!(to_frame(snap_to_frame(duration, fps), fps), frames);
        assert_eq!(timecode(3600.0, fps), "00:59:56:12");
    }

    #[test]
    fn fifty_cuts_keep_source_and_record_lengths_equal() {
        let fps = 30_000.0 / 1_001.0;
        let duration = 3172.135633;
        let spans: Vec<_> = retained_frame_spans(
            (0..=50).map(|index| {
                let start = f64::from(index) * 60.0;
                (start, start + 58.25)
            }),
            duration, fps,
        ).collect();
        assert_eq!(spans.len(), 51);
        let mut record = 0;
        for (source_in, source_out, timeline_in, timeline_out) in spans {
            assert_eq!(timeline_in, record);
            assert_eq!(source_out - source_in, timeline_out - timeline_in);
            record = timeline_out;
        }
        assert_eq!(record, 89_036);
    }

    #[test]
    fn kept_and_removed_frames_account_for_source() {
        let mut seed = 1_u64;
        for fps in [24.0, 25.0, 30_000.0 / 1_001.0, 30.0, 60_000.0 / 1_001.0] {
            let source_frames = 1000;
            for _ in 0..100 {
                let mut boundaries = Vec::new();
                for _ in 0..20 {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    boundaries.push(((seed >> 32) % source_frames as u64) as i32);
                }
                boundaries.sort_unstable();
                let mut end = 0;
                let mut removed = 0;
                let mut kept = Vec::new();
                for cut in boundaries.chunks_exact(2) {
                    kept.push((f64::from(end) / fps, f64::from(cut[0]) / fps));
                    removed += cut[1] - cut[0];
                    end = cut[1];
                }
                let duration = f64::from(source_frames) / fps;
                kept.push((f64::from(end) / fps, duration));
                let expected: Vec<_> = std::iter::once(0)
                    .chain(boundaries.iter().skip(1).step_by(2).copied())
                    .zip(boundaries.iter().step_by(2).copied().chain([source_frames]))
                    .filter(|(start, end)| start < end)
                    .map(|(start, end)| (i64::from(start), i64::from(end)))
                    .collect();
                let spans: Vec<_> = retained_frame_spans(kept, duration, fps).collect();
                assert_eq!(spans.iter().map(|&(a, b, _, _)| (a, b)).collect::<Vec<_>>(), expected);
                let mut source_end = 0;
                let mut timeline_end = 0;
                for (a, b, c, d) in spans {
                    assert!(a >= source_end && b > a && b <= i64::from(source_frames));
                    assert_eq!(c, timeline_end);
                    assert_eq!(b - a, d - c);
                    source_end = b;
                    timeline_end = d;
                }
                assert_eq!(timeline_end + i64::from(removed), i64::from(source_frames));
            }
        }
    }

    #[test]
    fn fifty_cuts_preserve_exact_endpoints_at_integer_rates() {
        for fps in [24.0, 25.0, 30.0] {
            let ranges = (0..=50).map(|index| {
                let source_in = index * 101;
                (f64::from(source_in) / fps, f64::from(source_in + 100) / fps)
            });
            let spans: Vec<_> = retained_frame_spans(ranges, 6000.0 / fps, fps).collect();
            assert_eq!(spans.len(), 51);
            for (index, span) in (0_i64..).zip(spans) {
                let source_in = index * 101;
                let timeline_in = index * 100;
                assert_eq!(span, (source_in, source_in + 100, timeline_in, timeline_in + 100));
            }
        }
    }
}
