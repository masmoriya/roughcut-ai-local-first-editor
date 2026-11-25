use super::Editor;
use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent};
use crate::model::SourceRange;
use uuid::Uuid;

pub(super) fn overlaps_protected(ranges: &[SourceRange], start: f64, end: f64) -> bool {
    ranges.iter().any(|range| range.start < end && start < range.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Project, Timeline};
    use crate::store::{SqliteStore, Store};

    #[test]
    fn saved_highlights_survive_reopening_project_storage() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("projects.sqlite");
        let mut project = Project::new("Transcript highlights");
        project.timeline = Timeline::new(10.0);
        project.highlighted_ranges = vec![SourceRange { start: 1.25, end: 2.5 }];
        SqliteStore::open(&database).unwrap().save_project(&project).unwrap();

        let reopened = SqliteStore::open(&database).unwrap().load_project(project.id).unwrap();
        assert_eq!(reopened.highlighted_ranges, project.highlighted_ranges);
        assert!(reopened.protected_ranges.is_empty());
        assert_eq!(reopened.timeline.duration, project.timeline.duration);
    }
}

pub(super) fn unprotected_parts(ranges: &[SourceRange], start: f64, end: f64) -> Vec<(f64, f64)> {
    let mut parts = Vec::new();
    let mut cursor = start;
    for range in ranges {
        if range.end <= cursor { continue; }
        if range.start >= end { break; }
        if range.start > cursor {
            parts.push((cursor, range.start));
        }
        cursor = cursor.max(range.end);
        if cursor >= end { break; }
    }
    if cursor < end {
        parts.push((cursor, end));
    }
    parts
}

fn frame_position(seconds: f64, fps: f64) -> f64 {
    let frame = seconds * fps;
    let nearest = frame.round();
    if (frame - nearest).abs() <= 1e-8 { nearest } else { frame }
}

impl Editor {
    pub fn protect_source_ranges(&self, project_id: Uuid, ranges: Vec<SourceRange>) -> Result<usize> {
        self.update_source_protection(project_id, ranges, false)
    }

    pub fn update_source_protection(&self, project_id: Uuid, ranges: Vec<SourceRange>, remove: bool) -> Result<usize> {
        self.update_source_annotation(project_id, ranges, remove, false, None, None, None)
    }

    pub fn update_source_highlights(&self, project_id: Uuid, ranges: Vec<SourceRange>, remove: bool) -> Result<usize> {
        self.update_source_annotation(project_id, ranges, remove, true, None, None, None)
    }

    pub(crate) fn update_source_annotation(&self, project_id: Uuid, mut ranges: Vec<SourceRange>, remove: bool, highlight: bool, expected_transcript: Option<Uuid>, expected_proposal: Option<Uuid>, split_cut: Option<u32>) -> Result<usize> {
        let _write = self.inner.proposal_writes.lock().unwrap();
        self.ensure_loaded(project_id)?;
        let preferences = split_cut.map(|_| self.get_preferences()).transpose()?;
        let count = {
            let mut state = self.inner.state.lock().unwrap();
            let entry = state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let project = entry.project.as_mut()
                .ok_or_else(|| CoreError::NotFound("project".into()))?;
            if expected_proposal.is_some_and(|expected| project.cut_proposal.as_ref().map(|proposal| proposal.id) != Some(expected)) {
                return Err(CoreError::Unavailable("The cut review changed. Select the passage again before protecting it.".into()));
            }
            if let Some(cut_id) = split_cut {
                let fps = project.fps();
                if !fps.is_finite() || fps <= 0.0 {
                    return Err(CoreError::InvalidArg("Source frame rate is unavailable".into()));
                }
                let selected = project.cut_proposal.as_ref()
                    .and_then(|proposal| proposal.cuts.iter().find(|cut| cut.id == cut_id && !cut.skipped));
                let matched = match (selected, ranges.as_slice()) {
                    (Some(cut), [range]) if range.start.is_finite() && range.end.is_finite() => cut.phrase_matches.iter().find(|matched| {
                        matched.source_in_frame == crate::time::to_frame(range.start, fps)
                            && matched.source_out_frame == crate::time::to_frame(range.end, fps)
                    }),
                    _ => None,
                };
                let Some(matched) = matched.filter(|_| !remove && !highlight && expected_proposal.is_some()) else {
                    return Err(CoreError::Unavailable("Select a phrase match in an active cut before protecting it.".into()));
                };
                ranges[0] = SourceRange {
                    start: matched.source_in_frame as f64 / fps,
                    end: (matched.source_out_frame as f64 / fps).min(project.timeline.duration),
                };
            }
            if let Some(expected) = expected_transcript {
                let transcript = entry.transcript.as_ref().filter(|transcript| transcript.id == expected)
                    .ok_or_else(|| CoreError::Unavailable("The transcript changed. Select the passage again before saving.".into()))?;
                super::media::validate_project_source(project)?;
                super::media::validate_transcript_source(transcript, project.media.as_ref())?;
            }
            let mut staged = project.clone();
            if ranges.iter().any(|range| !range.start.is_finite() || !range.end.is_finite()
                || range.start < 0.0 || range.start >= staged.timeline.duration
                || range.end <= range.start || range.end > staged.timeline.duration + 0.001)
            {
                return Err(CoreError::InvalidArg("Annotation ranges must be within the source".into()));
            }
            if !ranges.is_empty() {
                for range in &mut ranges {
                    range.end = range.end.min(staged.timeline.duration);
                }
                if !highlight {
                    let fps = staged.fps();
                    if !fps.is_finite() || fps <= 0.0 {
                        return Err(CoreError::InvalidArg("Source frame rate is unavailable".into()));
                    }
                    for range in &mut ranges {
                        range.start = frame_position(range.start, fps).floor() / fps;
                        range.end = (frame_position(range.end, fps).ceil() / fps).min(staged.timeline.duration);
                    }
                }
                ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
                if remove {
                    let previous_ranges = if highlight { &staged.highlighted_ranges } else { &staged.protected_ranges };
                    ranges = previous_ranges.iter().flat_map(|previous| {
                        unprotected_parts(&ranges, previous.start, previous.end).into_iter()
                            .map(|(start, end)| SourceRange { start, end })
                    }).collect();
                } else {
                    ranges.extend_from_slice(if highlight { &staged.highlighted_ranges } else { &staged.protected_ranges });
                    ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
                }
            } else if remove {
                return Ok(if highlight { staged.highlighted_ranges.len() } else { staged.protected_ranges.len() });
            }
            let target = if highlight { &mut staged.highlighted_ranges } else { &mut staged.protected_ranges };
            target.clear();
            for range in ranges {
                if let Some(previous) = target.last_mut().filter(|prev| range.start <= prev.end) {
                    previous.end = previous.end.max(range.end);
                } else {
                    target.push(range);
                }
            }
            let count = target.len();
            let fps = staged.fps();
            if let Some(proposal) = staged.cut_proposal.as_mut().filter(|_| !highlight) {
                if let Some((cut_id, prefs)) = split_cut.zip(preferences.as_ref()) {
                    super::proposals::split_protected_cut(proposal, cut_id, &staged.protected_ranges,
                        entry.transcript.as_ref(), &prefs.custom_filler_words, fps);
                }
                for cut in &mut proposal.cuts {
                    if overlaps_protected(&staged.protected_ranges, cut.start, cut.end) {
                        cut.skipped = true;
                    }
                }
            }
            if staged.cut_proposal.as_ref().is_some_and(|proposal| proposal.cuts.is_empty()) {
                staged.cut_proposal = None;
            }
            staged.updated_at = chrono::Utc::now();
            self.inner.store.save_project(&staged)?;
            *project = staged;
            count
        };
        send(&self.inner.sink, CoreEvent::CutProposalChanged { project_id });
        Ok(count)
    }
}
