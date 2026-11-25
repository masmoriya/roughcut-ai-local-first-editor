//! THE mutation seam. Every edit — UI, local agent, or MCP client — is an
//! [`EditOp`] applied through [`Editor::apply_edit`]: frame-snap → apply →
//! record a [`JournalEntry`] carrying the exact inverse and redo → persist →
//! notify. Undo/redo replay the same `apply_op` the edit used, so history can
//! never drift from the original edit.

use super::{EditOutcome, Editor, JournalEntry, ProjectState, RoughCutReport};
use crate::detect;
use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent};
use crate::model::*;
use crate::time::snap_to_frame;
use chrono::Utc;
use uuid::Uuid;

const UNDO_CAP: usize = 100;

impl Editor {
    /// THE mutation seam: every edit — UI, local agent, or MCP client — is an
    /// [`EditOp`] applied here. Frame-snap → apply → record an EditAction
    /// carrying the op, its exact inverse, and its exact redo → persist the
    /// project and the undo journal → notify.
    pub fn apply_edit(
        &self,
        project_id: Uuid,
        op: EditOp,
        source: ActionSource,
    ) -> Result<EditOutcome> {
        self.apply_edit_labeled(project_id, op, source, None)
    }

    /// `label` overrides the auto-derived history description (e.g. "Restored
    /// checkpoint 'before aggressive cut'" instead of "Restore previous cut
    /// state").
    pub fn apply_edit_labeled(
        &self,
        project_id: Uuid,
        op: EditOp,
        source: ActionSource,
        label: Option<String>,
    ) -> Result<EditOutcome> {
        self.commit_edit(project_id, op, source, label, None, None)
    }

    pub(crate) fn commit_edit(
        &self, project_id: Uuid, op: EditOp, source: ActionSource, label: Option<String>,
        proposal_id: Option<Uuid>, expected_transcript: Option<Uuid>,
    ) -> Result<EditOutcome> {
        self.ensure_loaded(project_id)?;
        let prefs = self.inner.store.load_preferences()?;
        let outcome = {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            if let Some(id) = expected_transcript {
                let transcript = entry.transcript.as_ref().filter(|value| value.id == id)
                    .ok_or_else(|| CoreError::Unavailable("The transcript changed. Select the passage again before editing.".into()))?;
                let project = entry.project.as_ref().ok_or_else(|| CoreError::NotFound("project".into()))?;
                super::media::validate_project_source(project)?;
                super::media::validate_transcript_source(transcript, project.media.as_ref())?;
            }
            let mut staged = entry.clone();
            let ProjectState { project, transcript, undo, redo } = &mut staged;
            let project =
                project.as_mut().ok_or_else(|| CoreError::NotFound("project".into()))?;
            let mut rule_dictionary_fingerprint = None;
            let mut cut_review = None;
            if let Some(expected) = proposal_id {
                super::media::validate_project_source(project)?;
                let proposal = project.cut_proposal.as_ref().filter(|proposal| proposal.id == expected)
                    .ok_or_else(|| CoreError::Unavailable("The cut proposal changed. Review it again before applying.".into()))?;
                if let Some(expected) = &proposal.timeline_fingerprint {
                    if super::proposals::proposal_timeline_fingerprint(project)? != *expected {
                        return Err(CoreError::Unavailable("The timeline changed. Discard the old proposal and review the current timeline.".into()));
                    }
                }
                if proposal.transcript_id.is_some_and(|id| transcript.as_ref().map(|value| value.id) != Some(id)) {
                    return Err(CoreError::Unavailable("The transcript changed. Discard the old proposal and review the current transcript.".into()));
                }
                if let Some(transcript) = transcript.as_ref() {
                    super::media::validate_transcript_source(transcript, project.media.as_ref())?;
                }
                let checkpoint_name = format!("Before {}", proposal.title);
                rule_dictionary_fingerprint = proposal.rule_dictionary_fingerprint.clone();
                if let EditOp::CutRanges { ranges } = &op {
                    let approved: std::collections::HashSet<_> = ranges.iter()
                        .map(|range| (range.start.to_bits(), range.end.to_bits())).collect();
                    let fps = project.fps();
                    let mut words: Vec<_> = transcript.iter().flat_map(|value| &value.segments)
                        .filter(|segment| !segment.is_silence)
                        .flat_map(|segment| segment.words.iter().enumerate().map(move |(index, word)| {
                            ((word.start + word.end) / 2.0, segment.id, index, word.text.as_str(), word.confidence)
                        }))
                        .filter(|(time, _, _, _, _)| time.is_finite())
                        .collect();
                    words.sort_by(|a, b| a.0.total_cmp(&b.0));
                    // Cue-timed transcripts have text but no individual word IDs.
                    let mut context_words = words.clone();
                    context_words.extend(transcript.iter().flat_map(|value| &value.segments)
                        .filter(|segment| !segment.is_silence && segment.words.is_empty())
                        .filter_map(|segment| {
                            let time = (segment.start + segment.end) / 2.0;
                            time.is_finite().then_some((time, segment.id, 0, segment.text.as_str(), 0.0))
                        }));
                    context_words.sort_by(|a, b| a.0.total_cmp(&b.0));
                    cut_review = Some(crate::model::CutReviewReceipt {
                        proposal_id: proposal.id,
                        proposal_title: proposal.title.clone(),
                        transcript_id: proposal.transcript_id,
                        decisions: proposal.cuts.iter().map(|cut| crate::model::CutReviewDecision {
                            cut_id: cut.id,
                            phrase_matches: cut.phrase_matches.iter().cloned().map(|mut matched| {
                                let start = matched.source_in_frame as f64 / fps;
                                let end = matched.source_out_frame as f64 / fps;
                                let first = words.partition_point(|word| word.0 < start);
                                let last = words.partition_point(|word| word.0 < end);
                                let matched_words = words.get(first..last).unwrap_or_default();
                                matched.word_ids = matched_words.iter().map(|(_, segment, index, _, _)| format!("{segment}:{index}")).collect();
                                matched.min_word_confidence = matched_words.iter().map(|word| word.4)
                                    .filter(|score| score.is_finite() && *score > 0.0).reduce(f64::min);
                                matched.context_before = review_context(&context_words, start - 3.0, start, true);
                                matched.context_after = review_context(&context_words, end, end + 3.0, false);
                                matched
                            }).collect(),
                            source_cut_id: proposal.transcript_id.map(|transcript_id| {
                                let start = crate::time::to_frame(cut.start, fps);
                                let end = crate::time::to_frame(cut.end, fps);
                                let category = cut.rule_category.as_deref().unwrap_or(&cut.kind);
                                format!("{transcript_id}:{start}:{end}:{category}")
                            }),
                            kind: cut.kind.clone(),
                            rule_category: cut.rule_category.clone(),
                            min_word_confidence: cut.min_word_confidence,
                            source_in_frame: crate::time::to_frame(cut.start, fps),
                            source_out_frame: crate::time::to_frame(cut.end, fps),
                            word_ids: words.get(words.partition_point(|word| word.0 < cut.start)
                                ..words.partition_point(|word| word.0 < cut.end)).unwrap_or_default()
                                .iter().map(|(_, segment, index, _, _)| format!("{segment}:{index}")).collect(),
                            context_before: review_context(&context_words, cut.start - 3.0, cut.start, true),
                            context_after: review_context(&context_words, cut.end, cut.end + 3.0, false),
                            approved: !cut.skipped && approved.contains(&(cut.start.to_bits(), cut.end.to_bits())),
                        }).collect(),
                    });
                }
                project.cut_proposal = None;
                Self::record_checkpoint(project, &checkpoint_name, true);
            }
            let op = normalize_op(op, project.fps());
            let included_before = project.timeline.included_duration();
            let before_clips = project.timeline.clips.clone();
            let before_padding = project.timeline.global_padding;
            let mut split_ids = vec![];
            validate_range(&op, project.timeline.duration)?;
            apply_op(
                project,
                transcript.as_mut(),
                &op,
                &prefs,
                &mut split_ids,
                self.inner.rough_cut_peaks.lock().unwrap().take().as_deref(),
            )?;
            project.timeline.normalize();
            let removed_duration = (included_before - project.timeline.included_duration()).max(0.0);
            if proposal_id.is_some() && removed_duration <= f64::EPSILON {
                return Err(CoreError::InvalidArg("These cuts no longer remove any footage. Discard the review or select different cuts.".into()));
            }
            project.updated_at = Utc::now();
            let mut action = EditAction::new(source, op);
            action.rule_dictionary_fingerprint = rule_dictionary_fingerprint;
            action.cut_review = cut_review;
            if let Some(l) = label {
                action.description = l;
            }
            undo.push(JournalEntry {
                action: action.clone(),
                inverse: EditOp::SetClips { clips: before_clips, global_padding: before_padding },
                redo: EditOp::SetClips {
                    clips: project.timeline.clips.clone(),
                    global_padding: project.timeline.global_padding,
                },
            });
            if undo.len() > UNDO_CAP {
                undo.remove(0);
            }
            redo.clear();
            self.save_edit_state(project, transcript.as_ref(), undo, redo)?;
            let split_clips = project
                .timeline
                .clips
                .iter()
                .filter(|c| split_ids.contains(&c.id))
                .cloned()
                .collect();
            let outcome = EditOutcome { action, timeline: project.timeline.clone(), removed_duration, split_clips };
            *entry = staged;
            outcome
        };
        send(&self.inner.sink, CoreEvent::TimelineChanged { project_id });
        if proposal_id.is_some() {
            send(&self.inner.sink, CoreEvent::CutProposalChanged { project_id });
        }
        Ok(outcome)
    }

    fn save_edit_state(
        &self, project: &Project, transcript: Option<&Transcript>, undo: &[JournalEntry], redo: &[JournalEntry],
    ) -> Result<()> {
        let journal = serde_json::json!({ "undo": undo, "redo": redo }).to_string();
        self.inner.store.save_edit(project, transcript, &journal, None)
    }

    /// Detect fillers + repeated takes and PERSIST the flags — the same write
    /// path the rough cut uses, so analysis behaves identically from every
    /// caller (UI, agent, MCP).
    pub fn annotate_transcript(
        &self,
        project_id: Uuid,
    ) -> Result<(detect::FillerFindings, Vec<detect::TakeGroup>)> {
        self.ensure_loaded(project_id)?;
        let prefs = self.inner.store.load_preferences()?;
        let out = {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let t = entry
                .transcript
                .as_mut()
                .ok_or_else(|| CoreError::InvalidArg("no transcript".into()))?;
            let result = detect::annotate(t, &prefs.custom_filler_words);
            self.inner.store.save_transcript(project_id, t)?;
            result
        };
        send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });
        Ok(out)
    }

    /// Cut ONLY the dead-air the audio waveform confirms — never words,
    /// fillers, takes, or order. The single source of truth for silence
    /// removal: both `remove_silences` and story_edit's pre-pass call this,
    /// so the pure path and the narrative pre-pass can never drift.
    /// `transcript_only` candidates are skipped (no audio proof). Returns the
    /// applied actions (empty when there's no audio or no confirmed dead air).
    pub async fn cut_confirmed_silences(
        &self,
        project_id: Uuid,
        source: ActionSource,
    ) -> Result<Vec<EditAction>> {
        let mut actions = vec![];
        for r in self.confirmed_silences(project_id).await? {
            if let Ok(outcome) =
                self.apply_edit(project_id, EditOp::CutRange { start: r.start, end: r.end }, source)
            {
                actions.push(outcome.action);
            }
        }
        Ok(actions)
    }

    /// The dead air the audio waveform confirms (see `cut_confirmed_silences`),
    /// as cut ranges. Empty when there's no transcript, media or audio.
    pub async fn confirmed_silences(&self, project_id: Uuid) -> Result<Vec<SourceRange>> {
        self.ensure_loaded(project_id)?;
        // Read inputs, then decode peaks OUTSIDE the state lock (slow).
        let (transcript, duration, media) = self.with_project(project_id, |p, t| {
            super::media::validate_project_source(p)?;
            Ok((t.cloned(), p.timeline.duration, p.media.clone()))
        })?;
        let (Some(t), Some(media)) = (transcript, media) else { return Ok(vec![]) };
        if media.source_audio_tracks.is_some_and(|tracks| tracks > 1) && t.source_audio_stream_index.is_none() {
            return Err(CoreError::InvalidArg("This transcript has no selected dialogue track for pause review".into()));
        }
        super::media::validate_transcript_source(&t, Some(&media))?;
        let peaks = crate::adapters::video::load_stream_peaks(&media, t.source_audio_stream_index).await;
        self.with_project(project_id, |project, current| {
            let unchanged = current.is_some_and(|current| current.id == t.id)
                && project.media.as_ref().is_some_and(|current| current.id == media.id && current.file_path == media.file_path);
            if !unchanged {
                return Err(CoreError::Unavailable("The source or transcript changed while checking audio. Run pause review again.".into()));
            }
            super::media::validate_project_source(project)?;
            super::media::validate_transcript_source(&t, project.media.as_ref())
        })?;
        let Some(peaks) = peaks else {
            return Ok(vec![]); // no audio energy to confirm against — refuse to guess
        };
        let prefs = self.inner.store.load_preferences()?;
        let min = prefs.silence_min_duration_s.max(0.6);
        let cands = detect::silence_candidates(&t, duration, (min * 0.6).max(0.3));
        let confirmed: Vec<SourceRange> = detect::refine_silences(
            &cands,
            Some(&peaks),
            crate::adapters::video::PEAKS_PER_SECOND,
            min,
        )
        .into_iter()
        .filter(|r| r.source == "confirmed")
        .map(|r| SourceRange { start: r.start, end: r.end })
        .collect();
        Ok(confirmed)
    }

    /// The rough cut, as a proposal: works out the pauses and filler words to
    /// cut (Tier 1) and flags repeated takes (Tier 2 suggestions), reporting
    /// each step as progress, but cuts nothing unless `apply` — the user
    /// reviews the proposal first (`apply_cut_proposal`). Replaces any pending
    /// proposal. Transcribes first if needed.
    pub async fn generate_rough_cut(
        &self,
        project_id: Uuid,
        aggressiveness: Option<Aggressiveness>,
        apply: bool,
        source: ActionSource,
    ) -> Result<RoughCutReport> {
        use crate::events::ProgressTask;
        self.ensure_loaded(project_id)?;
        if self.get_transcript(project_id)?.is_none() {
            if !self.inference_enabled() {
                return Err(CoreError::InvalidArg("Import a timed transcript before reviewing pauses and fillers".into()));
            }
            self.transcribe(project_id, None).await?;
        }
        let step = |fraction: f64, message: &str| {
            send(
                &self.inner.sink,
                CoreEvent::progress(ProgressTask::RoughCut, Some(project_id), fraction, message),
            )
        };
        let prefs = self.inner.store.load_preferences()?;
        let aggr = aggressiveness.unwrap_or(prefs.cut_aggressiveness);
        // Filler and take flags first: they're analysis, not cuts, and the
        // suggestions below read them.
        step(0.05, "finding filler words and repeated takes");
        self.annotate_transcript(project_id)?;
        // Hybrid silences: real audio energy (best effort — no ffmpeg means
        // transcript-only detection).
        step(0.2, "reading the audio");
        let (media, stream, has_word_timings) = self.with_project(project_id, |p, transcript| {
            super::media::validate_project_source(p)?;
            Ok((p.media.clone(), transcript.and_then(|value| value.source_audio_stream_index),
                transcript.is_some_and(|value| value.segments.iter().any(|segment| !segment.words.is_empty()))))
        })?;
        let peaks = match media {
            Some(media) if media.source_audio_tracks.is_some_and(|tracks| tracks > 1) && stream.is_none() => {
                return Err(CoreError::InvalidArg("This transcript has no selected dialogue track for pause and filler review".into()));
            }
            Some(media) => crate::adapters::video::load_stream_peaks(&media, stream).await,
            None => None,
        };
        if !has_word_timings && !self.inference_enabled()
            && peaks.as_deref().and_then(detect::levels).is_none()
        {
            return Err(CoreError::Unavailable("Pause review needs usable audio levels for a caption-only transcript".into()));
        }
        step(0.6, "finding pauses and checking each cut against the audio");
        let title = match aggr {
            Aggressiveness::Natural => "Rough cut",
            Aggressiveness::Aggressive => "Rough cut (aggressive)",
        };
        let proposed = self.propose_ops(
            project_id,
            vec![EditOp::RoughCut { aggressiveness: aggr }],
            title,
            source,
            peaks.as_deref(),
            true,
        )?;
        // TIER 2 — reviewable suggestions (repeated takes + semantic
        // repetition), stashed on the project. Embeddings are best-effort
        // (no index → takes only).
        step(0.85, "looking for repeated takes");
        let embeddings = if self.inference_enabled() {
            self.inner.store.load_embeddings(project_id)?
                .map(|(_, v)| v.into_iter().collect::<std::collections::HashMap<_, _>>())
        } else {
            None
        };
        let suggestions = match self.get_transcript(project_id)? {
            Some(t) => detect::suggest_cuts(&t, embeddings.as_ref()),
            None => vec![],
        };
        let suggestion_count = suggestions.len() as u32;
        {
            let mut state = self.inner.state.lock().unwrap();
            if let Some(p) = state.get_mut(&project_id).and_then(|e| e.project.as_mut()) {
                p.suggestions = suggestions;
                self.inner.store.save_project(p)?;
            }
        }
        let mut report = RoughCutReport::from_cuts(proposed.proposal.as_ref().map(|p| p.cuts.as_slice()).unwrap_or(&[]));
        report.suggestions = suggestion_count;
        report.audio_checked = peaks.is_some();
        if apply && proposed.proposal.is_some() {
            let applied = self.apply_cut_proposal(project_id, source)?;
            report.applied = true;
            report.action = applied.action;
        }
        let done = if report.cut_count == 0 {
            "nothing to cut".to_string()
        } else if report.applied {
            format!("{} cuts made", report.cut_count)
        } else {
            format!("{} cuts proposed — review them, then apply", report.cut_count)
        };
        step(1.0, &done);
        Ok(report)
    }

    /// Read the pending rough-cut suggestions (Tier-2 flags).
    pub fn get_suggestions(&self, project_id: Uuid) -> Result<Vec<crate::model::CutSuggestion>> {
        self.with_project(project_id, |p, _| Ok(p.suggestions.clone()))
    }

    /// Accept one suggestion: apply it as a normal undoable cut, then drop it
    /// from the list. Returns the edit action.
    pub(crate) fn review_saved_suggestions(&self, project_id: Uuid, suggestion_id: Option<Uuid>) -> Result<super::proposals::ProposeOutcome> {
        let (transcript_id, segment_ids) = self.with_project(project_id, |project, transcript| {
            let transcript = transcript.ok_or_else(|| CoreError::Unavailable("Import a timed transcript before reviewing suggestions.".into()))?;
            let ids: Vec<_> = project.suggestions.iter()
                .filter(|suggestion| suggestion_id.is_none_or(|id| suggestion.id == id))
                .flat_map(|suggestion| suggestion.segment_ids.iter().copied()).collect();
            Ok((transcript.id, live_segment_ids(&ids, Some(transcript))))
        })?;
        if segment_ids.is_empty() {
            return Err(CoreError::Unavailable("No saved suggestions match this transcript.".into()));
        }
        self.propose_labeled_ops(project_id, vec![EditOp::CutSegments { segment_ids }],
            ("Saved suggestions", None, Some(transcript_id)), &[], ActionSource::Ui, None, false)
    }

    pub fn accept_suggestion(
        &self,
        project_id: Uuid,
        suggestion_id: Uuid,
        source: ActionSource,
    ) -> Result<EditAction> {
        // Only cut segment ids that still exist — a transcript edit since the
        // rough cut can leave a suggestion pointing at gone ids. Stale ones
        // are dropped (the suggestion still drains), never erroring the cut.
        let seg_ids = self.with_project(project_id, |p, t| {
            let s = p
                .suggestions
                .iter()
                .find(|s| s.id == suggestion_id)
                .ok_or_else(|| CoreError::NotFound(format!("suggestion {suggestion_id}")))?;
            Ok(live_segment_ids(&s.segment_ids, t))
        })?;
        let action = if seg_ids.is_empty() {
            None
        } else {
            Some(self.apply_edit(project_id, EditOp::CutSegments { segment_ids: seg_ids }, source)?.action)
        };
        self.remove_suggestions(project_id, &[suggestion_id])?;
        action.ok_or_else(|| {
            CoreError::InvalidArg("that suggestion was stale (transcript changed) — cleared".into())
        })
    }

    /// Accept ALL pending suggestions in one undoable batch. Stale ids are
    /// filtered out so one gone segment can't abort the whole batch; every
    /// listed suggestion is drained regardless.
    pub fn accept_all_suggestions(
        &self,
        project_id: Uuid,
        source: ActionSource,
    ) -> Result<(Option<EditAction>, u32)> {
        let (ids, seg_ids): (Vec<Uuid>, Vec<Uuid>) = self.with_project(project_id, |p, t| {
            let ids: Vec<Uuid> = p.suggestions.iter().map(|s| s.id).collect();
            let raw: Vec<Uuid> = p.suggestions.iter().flat_map(|s| s.segment_ids.clone()).collect();
            Ok((ids, live_segment_ids(&raw, t)))
        })?;
        let count = ids.len() as u32;
        let action = if seg_ids.is_empty() {
            None
        } else {
            Some(self.apply_edit(project_id, EditOp::CutSegments { segment_ids: seg_ids }, source)?.action)
        };
        // Drain ALL listed suggestions even if some were stale.
        self.remove_suggestions(project_id, &ids)?;
        Ok((action, count))
    }

    /// Dismiss a suggestion without cutting.
    pub fn dismiss_suggestion(&self, project_id: Uuid, suggestion_id: Uuid) -> Result<()> {
        self.remove_suggestions(project_id, &[suggestion_id])
    }

    fn remove_suggestions(&self, project_id: Uuid, ids: &[Uuid]) -> Result<()> {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(p) = state.get_mut(&project_id).and_then(|e| e.project.as_mut()) {
            p.suggestions.retain(|s| !ids.contains(&s.id));
            self.inner.store.save_project(p)?;
        }
        send(&self.inner.sink, CoreEvent::TimelineChanged { project_id });
        Ok(())
    }

    /// Undo a specific action set (one chat turn) — ONLY when they are
    /// still the newest journal entries, so a stale button can never revert
    /// unrelated later work.
    pub fn undo_actions(&self, project_id: Uuid, ids: &[Uuid]) -> Result<(usize, Timeline)> {
        if ids.is_empty() {
            return Err(CoreError::InvalidArg("action_ids must be non-empty".into()));
        }
        self.ensure_loaded(project_id)?;
        {
            let state = self.inner.state.lock().unwrap();
            let entry = state
                .get(&project_id)
                .ok_or_else(|| CoreError::NotFound(format!("project {project_id}")))?;
            let top: Vec<Uuid> =
                entry.undo.iter().rev().take(ids.len()).map(|j| j.action.id).collect();
            let want: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
            if top.len() != ids.len() || !top.iter().all(|id| want.contains(id)) {
                return Err(CoreError::InvalidArg(
                    "those actions are no longer the newest edits — undo them from the \
                     top bar instead (newer changes exist)"
                        .into(),
                ));
            }
        }
        let mut timeline = self.get_timeline(project_id)?;
        for _ in 0..ids.len() {
            let (_, tl) = self.step_history(project_id, true)?;
            timeline = tl;
        }
        Ok((ids.len(), timeline))
    }

    pub fn undo(&self, project_id: Uuid) -> Result<(Option<EditAction>, Timeline)> {
        self.step_history(project_id, true)
    }

    pub fn redo(&self, project_id: Uuid) -> Result<(Option<EditAction>, Timeline)> {
        self.step_history(project_id, false)
    }

    /// Maximum saved checkpoints; oldest AUTO snapshot is evicted first so a
    /// user's named save points survive.
    const CHECKPOINT_CAP: usize = 40;

    /// Save the current timeline as a named checkpoint (a full snapshot, not a
    /// journal pointer — it survives any later undo/redo/jump).
    pub fn create_checkpoint(
        &self,
        project_id: Uuid,
        name: &str,
        auto: bool,
    ) -> Result<crate::model::CheckpointSummary> {
        self.ensure_loaded(project_id)?;
        let mut state = self.inner.state.lock().unwrap();
        let p = state
            .get_mut(&project_id)
            .and_then(|e| e.project.as_mut())
            .ok_or_else(|| CoreError::NotFound("project".into()))?;
        let summary = Self::record_checkpoint(p, name, auto);
        self.inner.store.save_project(p)?;
        Ok(summary)
    }

    fn record_checkpoint(p: &mut Project, name: &str, auto: bool) -> CheckpointSummary {
        let cp = Checkpoint {
            id: Uuid::new_v4(),
            name: if name.trim().is_empty() { "Checkpoint".into() } else { name.trim().into() },
            created_at: Utc::now(),
            clips: p.timeline.clips.clone(),
            global_padding: p.timeline.global_padding,
            cut_count: p.timeline.cut_count,
            auto,
        };
        let summary = cp.summary();
        p.checkpoints.push(cp);
        while p.checkpoints.len() > Self::CHECKPOINT_CAP {
            // Evict the oldest AUTO checkpoint; if none, the oldest overall.
            let idx = p
                .checkpoints
                .iter()
                .position(|c| c.auto)
                .unwrap_or(0);
            p.checkpoints.remove(idx);
        }
        summary
    }

    /// Lean checkpoint list (no clip dumps).
    pub fn get_checkpoints(&self, project_id: Uuid) -> Result<Vec<crate::model::CheckpointSummary>> {
        self.with_project(project_id, |p, _| {
            Ok(p.checkpoints.iter().rev().map(|c| c.summary()).collect())
        })
    }

    /// Restore a checkpoint as a normal undoable edit (so the restore itself
    /// is in the history and can be undone).
    pub fn restore_checkpoint(
        &self,
        project_id: Uuid,
        checkpoint_id: Uuid,
        source: ActionSource,
    ) -> Result<EditAction> {
        let (clips, padding, name) = self.with_project(project_id, |p, _| {
            let c = p
                .checkpoints
                .iter()
                .find(|c| c.id == checkpoint_id)
                .ok_or_else(|| CoreError::NotFound(format!("checkpoint {checkpoint_id}")))?;
            Ok((c.clips.clone(), c.global_padding, c.name.clone()))
        })?;
        let outcome = self.apply_edit_labeled(
            project_id,
            EditOp::SetClips { clips, global_padding: padding },
            source,
            Some(format!("Restored checkpoint “{name}”")),
        )?;
        Ok(outcome.action)
    }

    pub fn delete_checkpoint(&self, project_id: Uuid, checkpoint_id: Uuid) -> Result<()> {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(p) = state.get_mut(&project_id).and_then(|e| e.project.as_mut()) {
            p.checkpoints.retain(|c| c.id != checkpoint_id);
            self.inner.store.save_project(p)?;
        }
        Ok(())
    }

    /// The full edit history, oldest → newest: every applied edit (the undo
    /// stack) followed by every undone edit still available to redo (the
    /// future). `current_index` is the newest APPLIED entry (-1 = at the
    /// original, before any edit). The "go back in time" data source.
    pub fn history(&self, project_id: Uuid) -> Result<crate::model::History> {
        use crate::model::{History, HistoryEntry};
        self.ensure_loaded(project_id)?;
        let state = self.inner.state.lock().unwrap();
        let entry =
            state.get(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
        let to_entry = |j: &JournalEntry, applied: bool| {
            let kept = j.action.cut_review.as_ref().map_or(0, |receipt| {
                receipt.decisions.iter().filter(|decision| !decision.approved).count()
            });
            let description = if kept == 0 {
                j.action.description.clone()
            } else {
                format!("{} · {kept} kept", j.action.description)
            };
            HistoryEntry {
                id: j.action.id,
                kind: j.action.kind.clone(),
                source: j.action.source,
                description,
                timestamp: j.action.timestamp,
                applied,
                has_cut_review: j.action.cut_review.is_some(),
            }
        };
        let mut entries: Vec<HistoryEntry> =
            entry.undo.iter().map(|j| to_entry(j, true)).collect();
        // redo.last() is the next to re-apply (chronologically just after the
        // current point), so reversed gives true chronological future order.
        entries.extend(entry.redo.iter().rev().map(|j| to_entry(j, false)));
        Ok(History { current_index: entry.undo.len() as i64 - 1, entries })
    }

    pub fn history_cut_review(&self, project_id: Uuid, action_id: Uuid, offset: usize) -> Result<Option<serde_json::Value>> {
        self.ensure_loaded(project_id)?;
        let state = self.inner.state.lock().unwrap();
        let entry = state.get(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
        let journal = entry.undo.iter().chain(&entry.redo)
            .find(|journal| journal.action.id == action_id)
            .ok_or_else(|| CoreError::NotFound("history action".into()))?;
        Ok(journal.action.cut_review.as_ref().map(|review| {
            let offset = offset.min(review.decisions.len());
            let end = offset.saturating_add(50).min(review.decisions.len());
            serde_json::json!({
                "proposal_id": review.proposal_id,
                "proposal_title": review.proposal_title,
                "transcript_id": review.transcript_id,
                "total": review.decisions.len(),
                "decisions": &review.decisions[offset..end],
            })
        }))
    }

    /// Jump to a point in history: undo/redo until `target` is the newest
    /// applied edit. `None` = the original state (undo everything). The
    /// snapshots make every landing exact.
    pub fn jump_to(&self, project_id: Uuid, target: Option<Uuid>) -> Result<Timeline> {
        self.ensure_loaded(project_id)?;
        // TARGET-DRIVEN, not length-driven: re-derive the direction from the
        // LIVE stacks each iteration so a concurrent edit between steps can't
        // strand us (a frozen `desired` length could spin forever after a
        // redo.clear, or land on the wrong entry after an eviction). Each step
        // moves one entry between stacks; the iteration cap is a paranoia
        // backstop (the journal is bounded by UNDO_CAP anyway).
        enum Step {
            Done,
            Undo,
            Redo,
            Vanished,
        }
        let max_steps = 2 * UNDO_CAP + 8;
        let mut stepped = false;
        for _ in 0..max_steps {
            let step = {
                let state = self.inner.state.lock().unwrap();
                let entry = state
                    .get(&project_id)
                    .ok_or_else(|| CoreError::NotFound("project".into()))?;
                match target {
                    None => {
                        if entry.undo.is_empty() {
                            Step::Done
                        } else {
                            Step::Undo
                        }
                    }
                    Some(id) => {
                        if entry.undo.last().map(|j| j.action.id) == Some(id) {
                            Step::Done
                        } else if entry.undo.iter().any(|j| j.action.id == id) {
                            // Target is applied but below the top → undo the
                            // newer entries until it's the newest.
                            Step::Undo
                        } else if entry.redo.iter().any(|j| j.action.id == id) {
                            Step::Redo // target is in the future → redo toward it
                        } else {
                            Step::Vanished // a concurrent edit cleared/evicted it
                        }
                    }
                }
            };
            match step {
                Step::Done => {
                    // Rough-cut suggestions describe a specific timeline state;
                    // after moving through time they no longer match. Drop them
                    // (re-run the rough cut to regenerate for the new state).
                    if stepped {
                        let mut state = self.inner.state.lock().unwrap();
                        if let Some(p) = state.get_mut(&project_id).and_then(|e| e.project.as_mut())
                        {
                            if !p.suggestions.is_empty() {
                                p.suggestions.clear();
                                let _ = self.inner.store.save_project(p);
                            }
                        }
                    }
                    return self.get_timeline(project_id);
                }
                Step::Undo => {
                    self.step_history(project_id, true)?;
                    stepped = true;
                }
                Step::Redo => {
                    self.step_history(project_id, false)?;
                    stepped = true;
                }
                Step::Vanished => {
                    return Err(CoreError::NotFound(format!(
                        "history entry {target:?} is no longer in the timeline (a newer edit \
                         discarded it)"
                    )))
                }
            }
        }
        Err(CoreError::Other("jump_to did not converge".into()))
    }

    fn step_history(
        &self,
        project_id: Uuid,
        is_undo: bool,
    ) -> Result<(Option<EditAction>, Timeline)> {
        self.ensure_loaded(project_id)?;
        let prefs = self.inner.store.load_preferences()?;
        let result = {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let mut staged = entry.clone();
            let ProjectState { project, transcript, undo, redo } = &mut staged;
            let project =
                project.as_mut().ok_or_else(|| CoreError::NotFound("project".into()))?;
            let popped = if is_undo { undo.pop() } else { redo.pop() };
            let outcome = match popped {
                Some(entry) => {
                    let replay = if is_undo { &entry.inverse } else { &entry.redo };
                    apply_op(project, transcript.as_mut(), replay, &prefs, &mut vec![], None)?;
                    project.timeline.normalize();
                    project.updated_at = Utc::now();
                    let action = entry.action.clone();
                    if is_undo {
                        redo.push(entry);
                    } else {
                        undo.push(entry);
                    }
                    self.save_edit_state(project, transcript.as_ref(), undo, redo)?;
                    (Some(action), project.timeline.clone())
                }
                None => (None, project.timeline.clone()),
            };
            *entry = staged;
            outcome
        };
        send(&self.inner.sink, CoreEvent::TimelineChanged { project_id });
        Ok(result)
    }
}

// ------------------------------------------------------------- op helpers

/// Bounds validation for range ops BEFORE mutating: clamped garbage
/// (start>end, both ends past EOF) used to no-op silently, journal a phantom
/// entry, and report success — the exact false-positive a small model cannot
/// repair from.
pub(super) fn validate_range(op: &EditOp, duration: f64) -> Result<()> {
    if let EditOp::CutRange { start, end } | EditOp::RestoreRange { start, end } = op {
        if *start >= *end || *start < 0.0 || *start >= duration {
            return Err(CoreError::InvalidArg(format!(
                "range {:.2}s–{:.2}s is invalid: need 0 <= start < end and start < {duration:.2}s (the source duration)",
                start, end
            )));
        }
    }
    Ok(())
}

/// Frame-snap an op's time fields (frame-exact internally, seconds at the
/// API edge) and fold linked padding. One place, every caller.
pub(super) fn normalize_op(op: EditOp, fps: f64) -> EditOp {
    match op {
        EditOp::CutRange { start, end } => {
            EditOp::CutRange { start: snap_to_frame(start, fps), end: snap_to_frame(end, fps) }
        }
        EditOp::RestoreRange { start, end } => {
            EditOp::RestoreRange { start: snap_to_frame(start, fps), end: snap_to_frame(end, fps) }
        }
        EditOp::TrimClip { clip_id, new_source_in, new_source_out } => EditOp::TrimClip {
            clip_id,
            new_source_in: snap_to_frame(new_source_in, fps),
            new_source_out: snap_to_frame(new_source_out, fps),
        },
        EditOp::SplitClip { clip_id, at_time } => {
            EditOp::SplitClip { clip_id, at_time: snap_to_frame(at_time, fps) }
        }
        EditOp::SetGlobalPadding { start_s, end_s, linked } => {
            let (start_s, end_s) = if linked { (start_s, start_s) } else { (start_s, end_s) };
            EditOp::SetGlobalPadding { start_s, end_s, linked }
        }
        other => other,
    }
}

/// Keep only ids the current transcript still has (suggestion segment_ids
/// can go stale after a transcript edit) — order/dedup preserved.
fn live_segment_ids(ids: &[Uuid], transcript: Option<&Transcript>) -> Vec<Uuid> {
    match transcript {
        Some(t) => ids.iter().copied().filter(|id| t.segment(*id).is_some()).collect(),
        None => vec![],
    }
}

fn resolve_segments(t: &Transcript, ids: &[Uuid]) -> Result<Vec<(Uuid, f64, f64)>> {
    ids.iter()
        .map(|id| {
            t.segment(*id)
                .map(|s| (s.id, s.start, s.end))
                .ok_or_else(|| CoreError::NotFound(format!("segment {id}")))
        })
        .collect()
}

/// Apply one op to the project — shared verbatim by apply_edit, undo, and
/// redo, so history replay can never drift from the original edit (and by
/// cut proposals, on a copy).
pub(super) fn apply_op(
    project: &mut Project,
    transcript: Option<&mut Transcript>,
    op: &EditOp,
    prefs: &Preferences,
    split_ids: &mut Vec<Uuid>,
    // Raw audio peaks for RoughCut's hybrid silence refinement; None for
    // journal replays (snapshots replay exactly, never re-detect).
    rough_cut_peaks: Option<&[u8]>,
) -> Result<()> {
    match op {
        EditOp::CutRange { start, end } => {
            project.timeline.set_range_included(*start, *end, false, ClipOrigin::Manual);
        }
        EditOp::RestoreRange { start, end } => {
            project.timeline.set_range_included(*start, *end, true, ClipOrigin::Manual);
        }
        EditOp::CutSegments { segment_ids } => {
            let t = transcript
                .as_deref()
                .ok_or_else(|| CoreError::InvalidArg("no transcript yet".into()))?;
            for (id, s, e) in resolve_segments(t, segment_ids)? {
                project.timeline.cut_linked(s, e, &[id], ClipOrigin::Manual);
            }
        }
        EditOp::RestoreSegments { segment_ids } => {
            let t = transcript
                .as_deref()
                .ok_or_else(|| CoreError::InvalidArg("no transcript yet".into()))?;
            for (_, s, e) in resolve_segments(t, segment_ids)? {
                project.timeline.set_range_included(s, e, true, ClipOrigin::Manual);
            }
        }
        EditOp::TrimClip { clip_id, new_source_in, new_source_out } => {
            project
                .timeline
                .trim_clip(*clip_id, *new_source_in, *new_source_out)
                .map_err(CoreError::InvalidArg)?;
        }
        EditOp::SplitClip { clip_id, at_time } => {
            let (l, r) =
                project.timeline.split_clip(*clip_id, *at_time).map_err(CoreError::InvalidArg)?;
            split_ids.extend([l, r]);
        }
        EditOp::SetGlobalPadding { start_s, end_s, linked } => {
            project.timeline.apply_padding(*start_s, *end_s, *linked);
        }
        EditOp::RoughCut { aggressiveness } => {
            let t = transcript.ok_or_else(|| CoreError::InvalidArg("no transcript".into()))?;
            let duration = project.timeline.duration;
            let outcome = detect::generate_rough_cut(
                t,
                duration,
                *aggressiveness,
                &prefs.custom_filler_words,
                prefs.silence_min_duration_s,
                rough_cut_peaks,
                crate::adapters::video::PEAKS_PER_SECOND,
            );
            // Padding FIRST, on the cuts already there. The rough cut's own
            // cuts carry their own margins (silences keep onset and tail
            // guards; fillers are cut at quiet points), so padding them too
            // would shrink each by start + end and leave slivers inside the
            // words they remove.
            let padding = prefs.default_padding;
            if padding.start_s > 0.0 || padding.end_s > 0.0 {
                project.timeline.apply_padding(padding.start_s, padding.end_s, padding.linked);
            }
            // COMPOSE with the existing cut: AI exclusions are added on top of
            // whatever is already cut (manual or MCP edits survive — and the
            // whole pass is still one undo step). No cut shorter than
            // MIN_CUT_S: that's a jump, not an edit.
            for c in outcome
                .timeline
                .clips
                .iter()
                .filter(|c| !c.included && c.duration() >= detect::MIN_CUT_S)
            {
                project.timeline.cut_linked(
                    c.source_in,
                    c.source_out,
                    &c.linked_segment_ids,
                    ClipOrigin::AiCut,
                );
            }
        }
        EditOp::CutRanges { ranges } => {
            for r in ranges {
                project.timeline.set_range_included(r.start, r.end, false, ClipOrigin::AiCut);
            }
        }
        EditOp::SetClips { clips, global_padding } => {
            project.timeline.clips = clips.clone();
            project.timeline.global_padding = *global_padding;
        }
    }
    Ok(())
}
fn review_context(words: &[(f64, Uuid, usize, &str, f64)], start: f64, end: f64, before: bool) -> String {
    let first = words.partition_point(|word| word.0 < start);
    let last = words.partition_point(|word| word.0 < end);
    let nearby = &words[first..last];
    let context = nearby.iter().map(|word| word.3).collect::<Vec<_>>().join(" ");
    if before {
        let tail: Vec<_> = context.trim().chars().rev().take(100).collect();
        tail.into_iter().rev().collect()
    } else {
        context.trim().chars().take(100).collect()
    }
}

#[cfg(test)]
mod saved_suggestion_tests {
    use super::*;

    #[tokio::test]
    async fn saved_suggestion_review_leaves_timeline_unchanged_and_respects_protection() {
        let editor = Editor::test_instance();
        let project = editor.create_project("Saved suggestion review", Some("/demo/footage.mp4")).await.unwrap();
        let transcript = editor.transcribe_audio_stream(project.id, None, None).await.unwrap();
        let segment = transcript.segments.iter().find(|segment| !segment.is_silence && segment.end - segment.start > 0.5).unwrap();
        let suggestion = crate::model::CutSuggestion {
            id: Uuid::new_v4(), segment_ids: vec![segment.id], start: segment.start, end: segment.end,
            reason: "repeated_take".into(), confidence: 1.0, duplicate_of: None, preview: segment.text.clone(),
        };
        let suggestion_id = suggestion.id;
        editor.inner.state.lock().unwrap().get_mut(&project.id).unwrap()
            .project.as_mut().unwrap().suggestions.push(suggestion);
        let before = editor.get_timeline(project.id).unwrap();
        assert!(!editor.review_saved_suggestions(project.id, Some(suggestion_id)).unwrap().added.is_empty());
        let after = editor.get_timeline(project.id).unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(before).unwrap());
        editor.discard_cut_proposal(project.id).unwrap();
        editor.protect_source_ranges(project.id, vec![crate::model::SourceRange {
            start: segment.start, end: segment.end,
        }]).unwrap();
        assert!(editor.review_saved_suggestions(project.id, Some(suggestion_id)).unwrap().added.is_empty());
    }
}
