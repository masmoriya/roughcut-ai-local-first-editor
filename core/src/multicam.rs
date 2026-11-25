//! Resolve timeline coordinates and offline review plans. No Resolve mutations.
use crate::error::{CoreError, Result};
use crate::export::timecode::parse_timecode;
use crate::model::{Media, Timeline};
use crate::time::to_frame;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SNAPSHOT_VERSION: u32 = 1;
const MAX_TIMELINE_FRAMES: i64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveTimelineSnapshot {
    pub schema_version: u32,
    pub project_id: String,
    pub timeline_id: String,
    pub timeline_name: String,
    pub revision_sha256: String,
    pub start_frame: i64,
    pub end_frame: i64,
    pub start_timecode: String,
    pub rate_num: u32,
    pub rate_den: u32,
    pub drop_frame: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveTimelineBinding {
    pub timeline: ResolveTimelineSnapshot,
    /// SHA-256 of the playback export from this exact timeline revision.
    pub playback_sha256: String,
    /// SHA-256 of the timed JSON or SRT using seconds from the export start.
    pub transcript_sha256: String,
    pub dialogue_stream_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRange {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolveCutPlan {
    pub source: ResolveTimelineBinding,
    pub cuts: Vec<FrameRange>,
    pub result_frames: i64,
    pub plan_sha256: String,
}

impl ResolveTimelineSnapshot {
    pub fn frame_rate(&self) -> f64 {
        f64::from(self.rate_num) / f64::from(self.rate_den)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SNAPSHOT_VERSION {
            return Err(invalid("Unsupported Resolve timeline snapshot version"));
        }
        if [&self.project_id, &self.timeline_id, &self.timeline_name]
            .iter()
            .any(|field| {
                field.trim().is_empty() || field.len() > 512 || field.chars().any(char::is_control)
            })
            || !is_digest(&self.revision_sha256)
            || !(self.rate_den == 1 && (1..=240).contains(&self.rate_num)
                || self.rate_den == 1001
                    && matches!(self.rate_num, 24_000 | 30_000 | 48_000 | 60_000 | 120_000))
            || self.start_frame < 0
            || self.end_frame > MAX_TIMELINE_FRAMES
            || self.end_frame <= self.start_frame
        {
            return Err(invalid("Invalid Resolve timeline identity or frame range"));
        }
        let nominal = self.frame_rate().round() as i64;
        if parse_timecode(&self.start_timecode, nominal, self.rate_den == 1001)
            != Some((self.start_frame, self.drop_frame))
        {
            return Err(invalid(
                "Resolve start timecode does not match the start frame",
            ));
        }
        Ok(())
    }
}

impl ResolveTimelineBinding {
    pub fn validate(&self) -> Result<()> {
        self.timeline.validate()?;
        if !is_digest(&self.playback_sha256) || !is_digest(&self.transcript_sha256) {
            return Err(invalid(
                "The Resolve binding needs playback and transcript SHA-256 hashes",
            ));
        }
        Ok(())
    }

    pub fn validate_playback(&self, media: &Media, sha256: &str) -> Result<()> {
        self.validate()?;
        let rate = self.timeline.frame_rate();
        if sha256 != self.playback_sha256
            || !media.duration.is_finite()
            || media.duration <= 0.0
            || !media.frame_rate.is_finite()
            || (media.frame_rate - rate).abs() > 0.000_001
            || media
                .average_frame_rate
                .is_none_or(|fps| !fps.is_finite() || (fps - rate).abs() > 0.000_001)
            || to_frame(media.duration, rate) != self.timeline.end_frame - self.timeline.start_frame
            || !media
                .audio_streams
                .iter()
                .any(|stream| stream.index == self.dialogue_stream_index)
        {
            return Err(invalid(
                "Playback export does not match the Resolve timeline binding",
            ));
        }
        Ok(())
    }

    pub fn verify_transcript(&self, bytes: &[u8], stream: Option<usize>) -> Result<()> {
        if stream != Some(self.dialogue_stream_index)
            || format!("{:x}", Sha256::digest(bytes)) != self.transcript_sha256
        {
            return Err(invalid(
                "The timed transcript or dialogue stream does not match the Resolve binding",
            ));
        }
        Ok(())
    }

    pub fn plan(
        &self,
        current: &ResolveTimelineSnapshot,
        timeline: &Timeline,
    ) -> Result<ResolveCutPlan> {
        self.validate()?;
        if current != &self.timeline {
            return Err(CoreError::Unavailable(
                "Resolve timeline changed. Bind the current revision before applying cuts.".into(),
            ));
        }
        let rate = self.timeline.frame_rate();
        let origin = self.timeline.start_frame;
        let duration = self.timeline.end_frame - origin;
        if !timeline.duration.is_finite() || to_frame(timeline.duration, rate) != duration {
            return Err(invalid(
                "Review duration does not match the Resolve timeline",
            ));
        }
        let mut ranges = Vec::new();
        for clip in timeline.clips.iter().filter(|clip| !clip.included) {
            if !clip.source_in.is_finite()
                || !clip.source_out.is_finite()
                || clip.source_in < 0.0
                || clip.source_out > timeline.duration
            {
                return Err(invalid("A reviewed cut is outside the Resolve timeline"));
            }
            let start = to_frame(clip.source_in, rate);
            let end = to_frame(clip.source_out, rate);
            if start >= end || end > duration {
                return Err(invalid(
                    "Each reviewed cut must span at least one Resolve frame",
                ));
            }
            ranges.push(FrameRange {
                start: origin + start,
                end: origin + end,
            });
        }
        ranges.sort_unstable_by_key(|range| range.start);
        let mut cuts: Vec<FrameRange> = Vec::new();
        for range in ranges {
            match cuts.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => cuts.push(range),
            }
        }
        let result_frames = duration
            - cuts
                .iter()
                .map(|range| range.end - range.start)
                .sum::<i64>();
        if cuts.is_empty() || result_frames <= 0 {
            return Err(invalid(
                "Review at least one cut and leave part of the timeline",
            ));
        }
        let plan_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&(self, &cuts))?));
        Ok(ResolveCutPlan {
            source: self.clone(),
            cuts,
            result_frames,
            plan_sha256,
        })
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn invalid(message: &str) -> CoreError {
    CoreError::InvalidArg(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipOrigin;

    fn binding() -> ResolveTimelineBinding {
        ResolveTimelineBinding {
            timeline: ResolveTimelineSnapshot {
                schema_version: 1,
                project_id: "project-id".into(),
                timeline_id: "timeline-id".into(),
                timeline_name: "Multicam edit".into(),
                revision_sha256: "a".repeat(64),
                start_frame: 107_892,
                end_frame: 108_892,
                start_timecode: "01:00:00;00".into(),
                rate_num: 30_000,
                rate_den: 1001,
                drop_frame: true,
            },
            playback_sha256: "b".repeat(64),
            transcript_sha256: format!("{:x}", Sha256::digest(b"reviewed transcript")),
            dialogue_stream_index: 1,
        }
    }

    #[test]
    fn merges_overlaps_in_original_resolve_frames_and_survives_reopen() {
        let binding = binding();
        let rate = binding.timeline.frame_rate();
        let mut timeline = Timeline::new(1000.0 / rate);
        timeline.cut_linked(10.0 / rate, 20.0 / rate, &[], ClipOrigin::Manual);
        timeline.cut_linked(15.0 / rate, 40.0 / rate, &[], ClipOrigin::Manual);
        timeline.cut_linked(41.0 / rate, 100.0 / rate, &[], ClipOrigin::Manual);
        let plan = binding.plan(&binding.timeline, &timeline).unwrap();
        assert_eq!(
            plan.cuts,
            vec![
                FrameRange {
                    start: 107_902,
                    end: 107_932
                },
                FrameRange {
                    start: 107_933,
                    end: 107_992
                },
            ]
        );
        assert_eq!(plan.result_frames, 911);
        let mut project = crate::model::Project::new("Resolve review");
        project.resolve_timeline = Some(binding);
        project.timeline = timeline;
        let reopened: crate::model::Project =
            serde_json::from_slice(&serde_json::to_vec(&project).unwrap()).unwrap();
        let binding = reopened.resolve_timeline.unwrap();
        assert_eq!(
            binding.plan(&binding.timeline, &reopened.timeline).unwrap(),
            plan
        );
    }

    #[test]
    fn rejects_changed_revision_and_invalid_drop_frame_label() {
        let binding = binding();
        let mut changed = binding.timeline.clone();
        changed.revision_sha256 = "d".repeat(64);
        assert_eq!(
            binding
                .plan(&changed, &Timeline::new(1.0))
                .unwrap_err()
                .code(),
            "unavailable"
        );
        changed.start_timecode = "01:01:00;00".into();
        assert!(changed.validate().is_err());
        changed = binding.timeline.clone();
        changed.schema_version += 1;
        assert!(changed.validate().is_err());
    }

    #[test]
    fn transcript_requires_matching_bytes_and_dialogue_stream() {
        let binding = binding();
        assert!(binding
            .verify_transcript(b"reviewed transcript", Some(1))
            .is_ok());
        assert!(binding
            .verify_transcript(b"reviewed transcript", Some(0))
            .is_err());
        assert!(binding
            .verify_transcript(b"different transcript", Some(1))
            .is_err());
    }
}
