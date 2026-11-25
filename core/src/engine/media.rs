//! Media import and transcription, including the background LLM cleanup +
//! semantic-index pass that follows a real transcription.

use super::Editor;
use crate::applog;
use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent, ProgressTask};
use crate::model::*;
use chrono::Utc;
use uuid::Uuid;

fn normalize_speaker(label: &str) -> Result<Option<String>> {
    let label = label.trim();
    if label.chars().count() > 80 || label.chars().any(char::is_control) {
        return Err(CoreError::InvalidArg("Speaker labels can contain up to 80 characters without control characters".into()));
    }
    Ok((!label.is_empty()).then(|| label.to_owned()))
}

impl Editor {
    pub async fn import_media(
        &self,
        project_id: Option<Uuid>,
        file_path: &str,
    ) -> Result<Media> {
        if let Some(pid) = project_id {
            self.require_no_clip_session(pid, "importing a video")?;
        }
        let (mut media, identity) = self.probe_attached_media(file_path).await?;
        let fingerprint = identity.as_ref().map(SourceIdentity::fingerprint).transpose()?;
        if let Some(pid) = project_id {
            self.ensure_loaded(pid)?;
            {
                let _proposal_write = self.inner.proposal_writes.lock().unwrap();
                let mut state = self.inner.state.lock().unwrap();
                let entry =
                    state.get_mut(&pid).ok_or_else(|| CoreError::NotFound("project".into()))?;
                let mut project = entry.project.clone().ok_or_else(|| CoreError::NotFound("project".into()))?;
                let mut rebound_transcript = None;
                if project.resolve_timeline.is_some() {
                    return Err(CoreError::InvalidArg("Open a new Resolve review to change the playback export".into()));
                }
                if project.clip_session.is_some() {
                    return Err(CoreError::Unavailable("Close the clip editor before relinking the source".into()));
                }
                if let Some(expected) = identity.as_ref() {
                    if !source_identity(&media.file_path).is_ok_and(|current| &current == expected) {
                        return Err(CoreError::Unavailable("The source changed while importing it. Try the import again.".into()));
                    }
                }
                match &project.media {
                    // The same video at a new path (moved or renamed): relink.
                    // Cuts, history, transcript and clips all still apply.
                    Some(current) if same_source(current, &media) => {
                        if let (Some(identity), Some(candidate)) = (identity.as_ref(), fingerprint.as_deref()) {
                            let saved_transcript = entry.transcript.as_ref().and_then(|value| value.source_fingerprint.as_deref());
                            let saved_project = project.media_fingerprint.as_deref();
                            let mut previous_identity = identity.clone();
                            previous_identity.path = current.file_path.as_str().into();
                            let mut original = saved_project.is_some() || saved_transcript.is_some();
                            if original {
                                for expected in [saved_project, saved_transcript].into_iter().flatten() {
                                    original &= if expected.starts_with("metadata-v1:") {
                                        previous_identity.fingerprint_for(expected)? == expected
                                    } else {
                                        candidate == expected
                                    };
                                }
                            } else {
                                original = media_fingerprint(&current.file_path).is_ok_and(|expected| expected == candidate);
                            }
                            if !original {
                                return Err(CoreError::InvalidArg("Could not verify this is the original source. Open it as a new project instead.".into()));
                            }
                            if saved_transcript != Some(candidate) {
                                rebound_transcript = entry.transcript.clone().map(|mut transcript| {
                                    transcript.source_fingerprint = Some(candidate.to_owned());
                                    transcript
                                });
                            }
                        }
                        media.id = current.id;
                    }
                    // A different video would leave every cut, undo step,
                    // checkpoint, transcript and saved clip describing the
                    // old one — refuse rather than silently invalidate them.
                    Some(current) => {
                        return Err(CoreError::InvalidArg(format!(
                            "this project already has a different video ({:.1}s, {}x{}); \
                             create a new project for it, or use append_media to add it to the end",
                            current.duration, current.width, current.height
                        )))
                    }
                    None => project.timeline = Timeline::new(media.duration),
                }
                project.media = Some(media.clone());
                project.media_fingerprint = fingerprint.clone();
                project.updated_at = Utc::now();
                if let Some(transcript) = rebound_transcript {
                    let journal = serde_json::json!({ "undo": &entry.undo, "redo": &entry.redo }).to_string();
                    self.inner.store.save_edit(&project, Some(&transcript), &journal, None)?;
                    entry.transcript = Some(transcript);
                } else {
                    self.inner.store.save_project(&project)?;
                }
                entry.project = Some(project);
            }
            send(&self.inner.sink, CoreEvent::TimelineChanged { project_id: pid });
            send(&self.inner.sink, CoreEvent::MediaAssetsChanged { project_id: pid });
            send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id: pid });
        }
        Ok(media)
    }

    pub async fn transcribe(&self, project_id: Uuid, language: Option<&str>) -> Result<Transcript> {
        self.transcribe_audio_stream(project_id, language, None).await
    }

    pub async fn transcribe_audio_stream(&self, project_id: Uuid, language: Option<&str>, stream: Option<usize>) -> Result<Transcript> {
        if !self.inference_enabled() && !crate::adapters::transcribe::whisper_native_enabled() {
            return Err(CoreError::Unavailable("Import a timed JSON or SRT transcript to edit this source.".into()));
        }
        let active = &self.inner.transcriptions;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut runs = active.lock().unwrap();
            if runs.contains_key(&project_id) {
                return Err(CoreError::Unavailable("Transcription is already running for this project".into()));
            }
            runs.insert(project_id, cancel.clone());
        }
        struct ActiveRun<'a>(&'a super::Inner, Uuid);
        impl Drop for ActiveRun<'_> {
            fn drop(&mut self) {
                self.0.transcriptions.lock().unwrap().remove(&self.1);
            }
        }
        let _run = ActiveRun(&self.inner, project_id);
        let result = self.transcribe_project(project_id, language, cancel, stream).await;
        if result.is_err() {
            send(&self.inner.sink, CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 1.0, "stopped"));
        }
        result
    }

    pub fn cancel_transcription(&self, project_id: Uuid) -> bool {
        let runs = self.inner.transcriptions.lock().unwrap();
        let Some(cancel) = runs.get(&project_id) else { return false };
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        true
    }

    async fn transcribe_project(
        &self, project_id: Uuid, language: Option<&str>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        stream: Option<usize>,
    ) -> Result<Transcript> {
        self.ensure_loaded(project_id)?;
        let media = self
            .with_project(project_id, |project, transcript| {
                validate_project_source(project)?;
                if !self.inference_enabled() && (project.resolve_timeline.is_some() || project.clip_session.is_some() || transcript.is_some()) {
                    return Err(CoreError::InvalidArg("Transcribe a main project without a transcript or Resolve binding".into()));
                }
                Ok(project.media.clone())
            })?
            .ok_or_else(|| CoreError::InvalidArg("project has no media; import first".into()))?;
        match media.source_audio_tracks {
            Some(0) => return Err(CoreError::InvalidArg("The source has no audio track to transcribe".into())),
            Some(tracks) if tracks > 1 && stream.is_none() => return Err(CoreError::InvalidArg(
                format!("The source has {tracks} audio tracks. Select the dialogue track to transcribe."),
            )),
            _ => {}
        }
        if stream.is_some_and(|index| !media.audio_streams.iter().any(|audio| audio.index == index)) {
            return Err(CoreError::InvalidArg("Select an audio stream from the source footage".into()));
        }
        let prefs = self.inner.store.load_preferences()?;
        let lang = language.map(|s| s.to_string()).unwrap_or(prefs.language);
        let demo = self.demo_mode();
        let source_before = if demo { None } else { Some(source_identity(&media.file_path)?) };
        send(&self.inner.sink, CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 0.0, "starting"));

        // The app log: start, then end or the stage that failed. A start
        // with neither after it was cut off (quit or crash).
        let started = std::time::Instant::now();
        let (engine, whisper_model) = if demo {
            ("mock", None)
        } else {
            let engine = if crate::adapters::transcribe::whisper_native_enabled() { "whisper-rs" } else { "whisper-cli" };
            let model = crate::adapters::transcribe::resolve_whisper_model()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            (engine, model)
        };
        applog::write(
            applog::Level::Info,
            applog::Event::TranscribeStart { project: project_id, media_s: media.duration, language: lang.clone(), engine, whisper_model },
        );
        let failed = |stage: &'static str, e: CoreError| {
            applog::write(
                applog::Level::Error,
                applog::Event::TranscribeFailed {
                    project: project_id,
                    stage,
                    ms: started.elapsed().as_millis() as u64,
                    code: e.code(),
                    error: applog::ErrText::from(&e),
                },
            );
            e
        };

        let mut transcript = if demo {
            self.inner
                .transcriber
                .transcribe(&media, "", &lang, Some(project_id), &self.inner.sink, cancel.clone())
                .await
                .map_err(|e| failed("whisper", e))?
        } else {
            let scratch = tempfile::tempdir().map_err(|e| failed("audio", e.into()))?;
            let wav = scratch.path().join("source.wav");
            let wav_str = wav.to_string_lossy().to_string();
            send(
                &self.inner.sink,
                CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 0.1, "extracting audio"),
            );
            self.inner.video.extract_audio_wav(&media, &wav_str, stream).await.map_err(|e| failed("audio", e))?;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(CoreError::Unavailable("Transcription cancelled".into()));
            }
            send(
                &self.inner.sink,
                CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 0.25, "transcribing on-device"),
            );
            let t = self.inner.transcriber.transcribe(&media, &wav_str, &lang, Some(project_id), &self.inner.sink, cancel.clone()).await;
            t.map_err(|e| failed("whisper", e))?
        };
        // Measured word times: whisper's are estimates, and every word-level
        // cut aims at them (alignment when it can, then the audio trim).
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CoreError::Unavailable("Transcription cancelled".into()));
        }
        transcript.source_audio_stream_index = stream.or_else(|| media.audio_streams.first().map(|audio| audio.index));
        if !demo {
            send(&self.inner.sink, CoreEvent::progress(
                ProgressTask::Transcribe, Some(project_id), 0.98, "Finishing word timings…",
            ));
            self.place_words(&mut transcript, &media, project_id).await;
        }

        {
            let _commit = self.inner.transcriptions.lock().unwrap();
            let _proposal_write = self.inner.proposal_writes.lock().unwrap();
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(CoreError::Unavailable("Transcription cancelled".into()));
            }
            let mut state = self.inner.state.lock().unwrap();
            let entry = state
                .get_mut(&project_id)
                .ok_or_else(|| failed("save", CoreError::NotFound("project".into())))?;
            let current_media = entry.project.as_ref().and_then(|p| p.media.as_ref());
            if !current_media.is_some_and(|current| current.id == media.id && current.file_path == media.file_path) {
                return Err(failed("save", CoreError::Unavailable(
                    "Source media changed during transcription. Retry with the current source.".into(),
                )));
            }
            if let Some(before) = source_before {
                let current = source_identity(&media.file_path).map_err(|e| failed("save", e))?;
                if before != current {
                    return Err(failed("save", CoreError::Unavailable(
                        "Source file changed during transcription. Retry after it is stable.".into(),
                    )));
                }
                transcript.source_fingerprint = Some(current.fingerprint()?);
            }
            self.inner.store.save_transcript(project_id, &transcript).map_err(|e| failed("save", e))?;
            entry.transcript = Some(transcript.clone());
            // Rough-cut suggestions reference the OLD segment ids — a fresh
            // transcribe replaces every id, so the old flags are stale.
            if let Some(p) = entry.project.as_mut() {
                if !p.suggestions.is_empty() {
                    p.suggestions.clear();
                    let _ = self.inner.store.save_project(p);
                }
            }
        }
        applog::write(
            applog::Level::Info,
            applog::Event::TranscribeEnd {
                project: project_id,
                ms: started.elapsed().as_millis() as u64,
                segments: transcript.segments.len(),
                model: transcript.model_used.clone(),
            },
        );
        send(&self.inner.sink, CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 1.0, "done"));
        send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });

        // Gemma post-pass (real transcriptions only): clean up casing /
        // punctuation / mis-hearings in the background; the UI refreshes via
        // transcript-changed when it lands. Timestamps are never touched.
        if self.inference_enabled() {
            let editor = self.clone();
            tokio::spawn(async move {
                if !demo {
                    if let Ok(n) = crate::agent::clean_transcript(&editor, project_id).await {
                        if n > 0 {
                            send(
                                &editor.inner.sink,
                                CoreEvent::progress(ProgressTask::Transcribe,
                                    Some(project_id),
                                    1.0,
                                    format!("polished {n} segment(s) with {}", editor.model_label()),
                                ),
                            );
                        }
                    }
                }
                // Semantic index AFTER cleanup so vectors match final text.
                let _ = editor.index_transcript(project_id).await;
            });
        }
        Ok(transcript)
    }

    pub(crate) fn import_timed_transcript(&self, project_id: Uuid, mut transcript: Transcript, confirmed_source: Option<(&str, Uuid)>, expected_fps: Option<f64>, expected_duration: Option<f64>, source_bytes: &[u8]) -> Result<()> {
        self.ensure_loaded(project_id)?;
        let runs = self.inner.transcriptions.lock().unwrap();
        if runs.contains_key(&project_id) {
            return Err(CoreError::Unavailable("Wait for transcription to stop before importing".into()));
        }
        let mut state = self.inner.state.lock().unwrap();
        let entry = state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
        let project = entry.project.as_ref().ok_or_else(|| CoreError::NotFound("project".into()))?;
        let media = project.media.as_ref().ok_or_else(|| CoreError::NotFound("source media".into()))?;
        validate_project_source(project)?;
        if let Some(binding) = &project.resolve_timeline {
            transcript.source_audio_stream_index.get_or_insert(binding.dialogue_stream_index);
            binding.verify_transcript(source_bytes, transcript.source_audio_stream_index)?;
        }
        if expected_fps.is_some_and(|fps| !fps.is_finite() || fps <= 0.0 || (fps - media.frame_rate).abs() > 0.001) {
            return Err(CoreError::InvalidArg("The transcript frame rate does not match the attached source".into()));
        }
        if expected_duration.is_some_and(|duration| !duration.is_finite() || duration <= 0.0
            || (duration - media.duration).abs() > 1.0 / media.frame_rate.max(1.0)) {
            return Err(CoreError::InvalidArg("The transcript source duration does not match the attached footage".into()));
        }
        if entry.transcript.is_some() || project.clip_session.is_some() {
            return Err(CoreError::InvalidArg("Import into a main project without a transcript".into()));
        }
        if transcript.source_fingerprint.is_none() {
            let Some((confirmed_path, confirmed_media_id)) = confirmed_source else {
                return Err(CoreError::InvalidArg("Confirm that the transcript timestamps match the attached source".into()));
            };
            if confirmed_path != media.file_path || confirmed_media_id != media.id {
                return Err(CoreError::Unavailable("The attached source changed. Confirm the transcript against the current source.".into()));
            }
            transcript.source_fingerprint = Some(source_identity(&media.file_path)?.fingerprint()?);
        }
        validate_transcript_source(&transcript, Some(media))?;
        if media.source_audio_tracks.is_some_and(|tracks| tracks > 1)
            && transcript.source_audio_stream_index.is_none()
        {
            return Err(CoreError::InvalidArg(
                "Choose the dialogue track that matches this transcript".into(),
            ));
        }
        if transcript.source_audio_stream_index.is_some_and(|index| {
            !media.audio_streams.is_empty() && !media.audio_streams.iter().any(|audio| audio.index == index)
        }) {
            return Err(CoreError::InvalidArg("The transcript references an audio stream missing from this source".into()));
        }
        let valid_range = |start: f64, end: f64| start.is_finite() && end.is_finite()
            && start >= 0.0 && end >= start && end <= media.duration + 0.001;
        if transcript.segments.is_empty() {
            return Err(CoreError::InvalidArg("The transcript has no segments".into()));
        }
        for (index, segment) in transcript.segments.iter().enumerate() {
            let out_of_order = index > 0 && transcript.segments[index - 1].start > segment.start;
            if out_of_order || !valid_range(segment.start, segment.end)
                || segment.start >= media.duration || segment.end <= segment.start
            {
                return Err(CoreError::InvalidArg(format!(
                    "Segment {} has invalid source timestamps", index + 1,
                )));
            }
            if segment.words.iter().any(|word| {
                !valid_range(word.start, word.end)
                    || word.start < segment.start - 0.001
                    || word.end > segment.end + 0.001
                    || !(0.0..=1.0).contains(&word.confidence)
            }) || segment.words.windows(2).any(|pair| pair[0].start > pair[1].start)
            {
                return Err(CoreError::InvalidArg(format!(
                    "Segment {} has invalid word timestamps or confidence", index + 1,
                )));
            }
        }
        transcript.id = Uuid::new_v4();
        transcript.media_id = media.id;
        for segment in &mut transcript.segments {
            segment.end = segment.end.min(media.duration);
            for word in &mut segment.words {
                word.start = word.start.clamp(segment.start, segment.end);
                word.end = word.end.clamp(segment.start, segment.end);
            }
            segment.speaker = segment.speaker.as_deref().map(normalize_speaker).transpose()?.flatten();
            segment.id = Uuid::new_v4();
            segment.take_group_id = None;
            segment.is_best_take = false;
        }
        self.inner.store.save_transcript(project_id, &transcript)?;
        entry.transcript = Some(transcript);
        drop(state);
        drop(runs);
        send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });
        Ok(())
    }

    pub fn correct_transcript_segment(
        &self, project_id: Uuid, segment_id: Uuid, original: &str, original_speaker: &str,
        text: &str, speaker: Option<&str>,
    ) -> Result<()> {
        let _proposal_write = self.inner.proposal_writes.lock().unwrap();
        self.ensure_loaded(project_id)?;
        let mut state = self.inner.state.lock().unwrap();
        let entry = state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
        let project = entry.project.as_ref().ok_or_else(|| CoreError::NotFound("project".into()))?;
        validate_project_source(project)?;
        let mut transcript = entry.transcript.clone()
            .ok_or_else(|| CoreError::NotFound("transcript".into()))?;
        validate_transcript_source(&transcript, project.media.as_ref())?;
        let segment = transcript.segments.iter_mut()
            .find(|segment| segment.id == segment_id && !segment.is_silence)
            .ok_or_else(|| CoreError::NotFound("speech segment".into()))?;
        if segment.text != original || segment.speaker.as_deref().unwrap_or("") != original_speaker {
            return Err(CoreError::InvalidArg("Transcript changed. Reopen the correction.".into()));
        }
        if let Some(speaker) = speaker {
            segment.speaker = normalize_speaker(speaker)?;
        }
        let words: Vec<_> = text.split_whitespace().collect();
        if words.is_empty() {
            return Err(CoreError::InvalidArg("Enter transcript text".into()));
        }
        if !segment.words.is_empty() && words.len() != segment.words.len() {
            return Err(CoreError::InvalidArg(
                "Keep the same number of words to preserve their timing.".into(),
            ));
        }
        if segment.words.is_empty() {
            if segment.text != text.trim() && segment.original_text.is_none() {
                segment.original_text = Some(segment.text.clone());
            }
            segment.text = text.trim().to_owned();
        } else if segment.words.iter().zip(&words).any(|(word, replacement)| word.text != *replacement) {
            if segment.original_text.is_none() {
                segment.original_text = Some(segment.text.clone());
            }
            for (word, replacement) in segment.words.iter_mut().zip(&words) {
                if word.text != *replacement && word.original_text.is_none() {
                    word.original_text = Some(word.text.clone());
                }
                word.text = (*replacement).to_owned();
            }
            segment.text = words.join(" ");
        }
        self.inner.store.save_transcript(project_id, &transcript)?;
        entry.transcript = Some(transcript);
        drop(state);
        send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });
        Ok(())
    }

    /// Replace segment texts (and word texts when counts align) keeping ALL
    /// timestamps intact — used by the LLM transcript cleanup pass.
    pub fn update_segment_texts(&self, project_id: Uuid, changes: &[(Uuid, String)]) -> Result<u32> {
        self.ensure_loaded(project_id)?;
        let mut applied = 0;
        {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let t = entry
                .transcript
                .as_mut()
                .ok_or_else(|| CoreError::InvalidArg("no transcript".into()))?;
            for (id, text) in changes {
                if let Some(seg) = t.segments.iter_mut().find(|s| s.id == *id) {
                    let new_words: Vec<&str> = text.split_whitespace().collect();
                    if new_words.len() == seg.words.len() {
                        for (w, nw) in seg.words.iter_mut().zip(&new_words) {
                            w.text = (*nw).to_string();
                        }
                    }
                    seg.text = text.clone();
                    applied += 1;
                }
            }
            if applied > 0 {
                t.model_used =
                    format!("{}+llm-cleanup", t.model_used.trim_end_matches("+llm-cleanup"));
                self.inner.store.save_transcript(project_id, t)?;
            }
        }
        if applied > 0 {
            send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });
        }
        Ok(applied)
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct SourceIdentity {
    path: std::path::PathBuf,
    bytes: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    file_id: (u64, u64),
}

impl Editor {
    pub fn media_metadata_matches(path: &std::path::Path, metadata: &std::fs::Metadata, expected: &str) -> bool {
        SourceIdentity::from_metadata(path.to_path_buf(), metadata)
            .and_then(|identity| identity.fingerprint_for(expected))
            .is_ok_and(|actual| actual == expected)
    }

    pub(super) async fn probe_attached_media(&self, path: &str) -> Result<(Media, Option<SourceIdentity>)> {
        if self.demo_mode() {
            return Ok((self.inner.video.probe(path).await?, None));
        }
        let identity = source_identity(path)?;
        let canonical = identity.path.to_str().ok_or_else(|| {
            CoreError::InvalidArg("The source filename is not valid Unicode".into())
        })?;
        let media = self.inner.video.probe(canonical).await?;
        if !source_identity(path).is_ok_and(|after| after == identity) {
            return Err(CoreError::Unavailable("Source media changed during import. Choose the file again.".into()));
        }
        Ok((media, Some(identity)))
    }

    pub(crate) async fn probe_export_source(&self, file_path: &str, expected_fingerprint: Option<&str>) -> Result<(Media, String)> {
        use crate::adapters::video::{FfmpegCli, VideoEngine};

        let before = source_identity(file_path).map_err(|_| {
            CoreError::NotFound(format!("Relink the source media before exporting: {file_path}"))
        })?;
        if let Some(expected) = expected_fingerprint {
            if before.fingerprint_for(expected)? != expected {
                return Err(CoreError::Unavailable("Source media changed. Relink the original media before exporting.".into()));
            }
        }
        let path = before.path.to_str().ok_or_else(|| {
            CoreError::InvalidArg("The source filename is not valid Unicode".into())
        })?;
        let mut media = FfmpegCli.probe(path).await?;
        if !source_identity(file_path).is_ok_and(|after| after == before) {
            return Err(CoreError::Unavailable("Source media changed during export. Relink it and try again.".into()));
        }
        media.file_path = path.to_owned();
        Ok((media, before.fingerprint()?))
    }

    pub(crate) fn verify_export_source(&self, path: &str, expected: &str) -> Result<()> {
        if media_fingerprint(path).is_ok_and(|actual| actual == expected) {
            return Ok(());
        }
        Err(CoreError::Unavailable("Source media changed during export. Relink it and try again.".into()))
    }
}

impl SourceIdentity {
    pub(super) fn content_sha256(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;

        let mut file = std::fs::File::open(&self.path)?;
        let matches = || CoreError::Unavailable("Playback changed during verification. Choose the export again.".into());
        if &Self::from_metadata(self.path.clone(), &file.metadata()?)? != self {
            return Err(matches());
        }
        let mut digest = Sha256::new();
        let bytes = std::io::copy(&mut (&mut file).take(self.bytes.saturating_add(1)), &mut digest)?;
        if bytes != self.bytes || &Self::from_metadata(self.path.clone(), &file.metadata()?)? != self {
            return Err(matches());
        }
        Ok(format!("{:x}", digest.finalize()))
    }

    fn from_metadata(path: std::path::PathBuf, metadata: &std::fs::Metadata) -> Result<Self> {
        if !metadata.is_file() {
            return Err(CoreError::InvalidArg("The source must be a local file".into()));
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path,
            bytes: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            file_id: (metadata.dev(), metadata.ino()),
        })
    }

    fn fingerprint_for(&self, expected: &str) -> Result<String> {
        if expected.starts_with("metadata-v1:") {
            use sha2::{Digest, Sha256};
            return Ok(format!("metadata-v1:{:x}", Sha256::digest(serde_json::to_vec(self)?)));
        }
        self.fingerprint()
    }

    pub(super) fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        #[cfg(unix)]
        let identity = (self.bytes, self.modified, self.file_id);
        #[cfg(not(unix))]
        let identity = (self.bytes, self.modified, &self.path);
        Ok(format!("metadata-v2:{:x}", Sha256::digest(serde_json::to_vec(&identity)?)))
    }
}

pub(super) fn media_fingerprint(path: &str) -> Result<String> {
    source_identity(path)?.fingerprint()
}

pub(super) fn validate_project_source(project: &crate::model::Project) -> Result<()> {
    if let Some(expected) = project.media_fingerprint.as_deref() {
        let current = project.media.as_ref()
            .and_then(|media| source_identity(&media.file_path).ok())
            .map(|identity| identity.fingerprint_for(expected))
            .transpose()?;
        if current.as_deref() != Some(expected) {
            return Err(CoreError::Unavailable("The source changed or is offline. Relink the original media.".into()));
        }
    }
    Ok(())
}

pub(super) fn validate_transcript_source(transcript: &Transcript, media: Option<&Media>) -> Result<()> {
    let Some(expected) = &transcript.source_fingerprint else { return Ok(()) };
    let actual = media.and_then(|media| source_identity(&media.file_path).ok())
        .map(|identity| identity.fingerprint_for(expected))
        .transpose()?;
    if actual.as_ref() != Some(expected) {
        return Err(CoreError::Unavailable("Source media changed. Relink the original media.".into()));
    }
    Ok(())
}

fn source_identity(path: &str) -> Result<SourceIdentity> {
    let path = std::path::Path::new(path).canonicalize()?;
    let metadata = path.metadata()?;
    SourceIdentity::from_metadata(path, &metadata)
}

/// What fixing word times did (`retime_words`, and after transcription).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WordPlacement {
    /// Forced alignment placed the words.
    pub aligned: bool,
    pub chunks: usize,
    /// Chunks that kept whisper's times (the model's words didn't match).
    pub chunks_kept: usize,
    #[serde(flatten)]
    pub trim: crate::detect::TrimStats,
    /// Why alignment didn't run, when it didn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl super::Editor {
    /// Fix word timings on an existing transcript, as transcription now
    /// does: forced alignment against the audio when this build and the model
    /// can (English), then the audio trim. Changes no text, segment order or
    /// cut.
    pub async fn retime_words(&self, project_id: Uuid) -> Result<WordPlacement> {
        self.ensure_loaded(project_id)?;
        let media = self
            .with_project(project_id, |p, _| {
                if p.cut_proposal.is_some() {
                    return Err(CoreError::Unavailable("Apply or discard the pending cuts before fixing word timings.".into()));
                }
                Ok(p.media.clone())
            })?
            .ok_or_else(|| CoreError::InvalidArg("project has no media".into()))?;
        let mut transcript = self
            .get_transcript(project_id)?
            .ok_or_else(|| CoreError::InvalidArg("project has no transcript; transcribe first".into()))?;
        let source_before = source_identity(&media.file_path)?;
        if media.source_audio_tracks.is_some_and(|tracks| tracks > 1) && transcript.source_audio_stream_index.is_none() {
            return Err(CoreError::InvalidArg("This transcript has no selected dialogue track".into()));
        }
        if crate::adapters::video::load_stream_peaks(&media, transcript.source_audio_stream_index).await.is_none() {
            return Err(CoreError::Unavailable("fixing word timings reads the audio, which needs ffmpeg".into()));
        }
        let placement = self.place_words(&mut transcript, &media, project_id).await;
        {
            let mut state = self.inner.state.lock().unwrap();
            let entry = state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            if entry.project.as_ref().is_some_and(|project| project.cut_proposal.is_some()) {
                return Err(CoreError::Unavailable("A cut proposal was created while aligning words. Review it before retrying.".into()));
            }
            let current_media = entry.project.as_ref().and_then(|project| project.media.as_ref())
                .filter(|current| current.id == media.id && current.file_path == media.file_path)
                .ok_or_else(|| CoreError::Unavailable("Source media changed while aligning words".into()))?;
            let source_after = source_identity(&current_media.file_path)?;
            if source_before != source_after {
                return Err(CoreError::Unavailable("Source media changed while aligning words".into()));
            }
            let mut current = entry.transcript.clone().filter(|current| current.id == transcript.id)
                .ok_or_else(|| CoreError::Unavailable("Transcript was replaced while aligning words".into()))?;
            validate_transcript_source(&current, Some(current_media))?;
            if current.segments.len() != transcript.segments.len() ||
                current.segments.iter().zip(&transcript.segments)
                    .any(|(dst, src)| dst.id != src.id || dst.words.len() != src.words.len())
            {
                return Err(CoreError::Unavailable("Transcript structure changed while aligning words".into()));
            }
            for (dst, src) in current.segments.iter_mut().zip(&transcript.segments) {
                dst.start = src.start;
                dst.end = src.end;
                for (dw, sw) in dst.words.iter_mut().zip(&src.words) {
                    dw.start = sw.start;
                    dw.end = sw.end;
                }
            }
            current.words_trimmed |= transcript.words_trimmed;
            current.words_aligned |= transcript.words_aligned;
            current.source_fingerprint = Some(source_after.fingerprint()?);
            self.inner.store.save_transcript(project_id, &current)?;
            entry.transcript = Some(current);
        }
        send(&self.inner.sink, CoreEvent::TranscriptChanged { project_id });
        Ok(placement)
    }

    /// Measured word times for `transcript`: forced alignment (when the
    /// `word-align` build has its model and the language is English), then
    /// the audio trim. Best effort: whatever can't run is skipped and
    /// reported in `note`, never an error.
    async fn place_words(&self, transcript: &mut Transcript, media: &Media, project_id: Uuid) -> WordPlacement {
        use crate::adapters::align;
        let mut out = WordPlacement::default();
        if !align::aligner_compiled() {
            out.note = Some("this build has no word aligner".into());
        } else if !align::aligner_downloaded() {
            out.note = Some("download the word alignment model in Setup for measured word times".into());
        } else if !align::language_supported(&transcript.language) {
            out.note = Some(format!("word alignment is English-only; this transcript is '{}'", transcript.language));
        } else {
            send(&self.inner.sink, CoreEvent::progress(ProgressTask::Transcribe, Some(project_id), 0.98, "aligning words to the audio"));
            match align::read_audio_16k(&media.file_path, transcript.source_audio_stream_index).await {
                Ok(samples) => {
                    let mut t = transcript.clone();
                    let sink = self.sink();
                    let duration = media.duration;
                    let run = tokio::task::spawn_blocking(move || {
                        let r = align::align_transcript(&mut t, &samples, duration, &|f| {
                            send(
                                &sink,
                                CoreEvent::progress(
                                    ProgressTask::Transcribe,
                                    Some(project_id),
                                    0.98 + 0.019 * f,
                                    format!("aligning words to the audio — {:.0}%", f * 100.0),
                                ),
                            );
                        });
                        (t, r)
                    })
                    .await;
                    match run {
                        Ok((t, Ok(stats))) => {
                            *transcript = t;
                            transcript.words_aligned = stats.aligned > 0;
                            out.aligned = transcript.words_aligned;
                            out.chunks = stats.chunks;
                            out.chunks_kept = stats.kept;
                        }
                        Ok((_, Err(e))) => out.note = Some(format!("word alignment failed: {e}")),
                        Err(e) => out.note = Some(format!("word alignment stopped: {e}")),
                    }
                }
                Err(e) => out.note = Some(e.to_string()),
            }
        }
        if let Some(peaks) = crate::adapters::video::load_stream_peaks(media, transcript.source_audio_stream_index).await {
            out.trim = crate::detect::trim_words_to_audio(transcript, &peaks, crate::adapters::video::PEAKS_PER_SECOND);
            transcript.words_trimmed = true;
        }
        out
    }

    /// Append another video to the END of this project's source: lossless
    /// concat into a new file in `<data dir>/media/` (originals untouched),
    /// media swapped, timeline extended with an included clip over the new
    /// span. Existing cuts and transcript timings keep their source positions.
    /// Inputs must match codec/resolution (stream copy) — mismatches refuse.
    pub async fn append_media(&self, project_id: uuid::Uuid, file_path: &str) -> crate::error::Result<crate::model::Media> {
        self.require_single_file_project(project_id)?;
        use crate::error::CoreError;
        self.require_no_clip_session(project_id, "adding a video")?;
        let (current, expected_fingerprint) = self.with_project(project_id, |project, transcript| {
            let media = project.media.clone().ok_or_else(|| CoreError::InvalidArg("project has no media".into()))?;
            let expected = project.media_fingerprint.clone()
                .or_else(|| transcript.and_then(|value| value.source_fingerprint.clone()));
            Ok((media, expected))
        })?;
        if !std::path::Path::new(file_path).is_file() {
            return Err(CoreError::NotFound(format!("file {file_path}")));
        }
        let original_identity = source_identity(&current.file_path)?;
        if let Some(expected) = expected_fingerprint.as_deref() {
            if original_identity.fingerprint_for(expected)? != expected {
                return Err(CoreError::Unavailable("The original source changed. Relink it before adding footage.".into()));
            }
        }
        let appended_identity = source_identity(file_path)?;
        // concat_media carries the codec/resolution guard.
        let combined = crate::adapters::video::concat_media(&[
            current.file_path.clone(),
            file_path.to_string(),
        ])
        .await?;
        let new_media = match self.video().probe(&combined).await {
            Ok(media) => media,
            Err(error) => {
                let _ = std::fs::remove_file(&combined);
                return Err(error);
            }
        };
        let span = crate::model::Clip {
            id: uuid::Uuid::new_v4(),
            source_in: current.duration,
            source_out: new_media.duration,
            included: true,
            origin: crate::model::ClipOrigin::Manual,
            order: 0,
            linked_segment_ids: vec![],
        };
        let committed = (|| -> Result<()> {
            let _proposal_write = self.inner.proposal_writes.lock().unwrap();
            let mut state = self.inner.state.lock().unwrap();
            let entry = state
                .get_mut(&project_id)
                .ok_or_else(|| CoreError::NotFound(format!("project {project_id}")))?;
            let mut staged = entry.clone();
            extend_history(&mut staged, &span);
            let project = staged
                .project
                .as_mut()
                .ok_or_else(|| CoreError::NotFound(format!("project {project_id}")))?;
            if project.clip_session.is_some()
                || !project.media.as_ref().is_some_and(|media| media.id == current.id && media.file_path == current.file_path)
                || source_identity(&current.file_path)? != original_identity
                || source_identity(file_path)? != appended_identity
            {
                return Err(CoreError::Unavailable("The source or edit session changed while adding footage. Retry the append.".into()));
            }
            if !new_media.duration.is_finite() || new_media.duration <= current.duration {
                return Err(CoreError::Unavailable("The combined footage has no new playable duration".into()));
            }
            if let Some(transcript) = staged.transcript.as_mut() {
                validate_transcript_source(transcript, Some(&current))?;
                transcript.media_id = new_media.id;
                transcript.source_fingerprint = Some(source_identity(&new_media.file_path)?.fingerprint()?);
            }
            project.timeline.duration = new_media.duration;
            push_span(&mut project.timeline.clips, &span);
            project.timeline.normalize();
            project.media = Some(new_media.clone());
            project.media_fingerprint = Some(media_fingerprint(&new_media.file_path)?);
            let journal = serde_json::json!({ "undo": &staged.undo, "redo": &staged.redo }).to_string();
            self.inner.store.save_edit(project, staged.transcript.as_ref(), &journal, None)?;
            *entry = staged;
            Ok(())
        })();
        if let Err(error) = committed {
            let _ = std::fs::remove_file(&combined);
            return Err(error);
        }
        crate::events::send(
            &self.sink(),
            crate::events::CoreEvent::TimelineChanged { project_id },
        );
        crate::events::send(
            &self.sink(),
            crate::events::CoreEvent::MediaAssetsChanged { project_id },
        );
        Ok(new_media)
    }
}

impl super::Editor {
    /// Attach a screen file that started with the camera file (probed; offset 0).
    pub async fn attach_screen_media(&self, project_id: uuid::Uuid, file_path: &str) -> crate::error::Result<()> {
        self.require_single_file_project(project_id)?;
        self.require_no_clip_session(project_id, "adding a screen recording")?;
        let (media, identity) = self.probe_attached_media(file_path).await?;
        let fingerprint = identity.as_ref().map(SourceIdentity::fingerprint).transpose()?;
        self.ensure_loaded(project_id)?;
        let mut state = self.inner.state.lock().unwrap();
        let entry = state
            .get_mut(&project_id)
            .and_then(|e| e.project.as_mut())
            .ok_or_else(|| crate::error::CoreError::NotFound(format!("project {project_id}")))?;
        if entry.clip_session.is_some() {
            return Err(CoreError::Unavailable("Close the clip editor before adding a screen recording".into()));
        }
        if let Some(expected) = identity.as_ref() {
            if !source_identity(&media.file_path).is_ok_and(|current| &current == expected) {
                return Err(CoreError::Unavailable("The screen recording changed while importing it. Try the import again.".into()));
            }
        }
        let mut project = entry.clone();
        project.screen_media = Some(media);
        project.screen_media_fingerprint = fingerprint;
        self.inner.store.save_project(&project)?;
        *entry = project;
        Ok(())
    }
}

/// Is `b` the same video as `a` (e.g. the file was moved)? Duration, frame
/// size and frame rate must match.
fn same_source(a: &Media, b: &Media) -> bool {
    (a.duration - b.duration).abs() < 0.05
        && a.width == b.width
        && a.height == b.height
        && (a.frame_rate - b.frame_rate).abs() < 0.01
}

/// Append `span` (an included clip over newly appended footage) to a clip list.
fn push_span(clips: &mut Vec<Clip>, span: &Clip) {
    let mut clip = span.clone();
    clip.order = clips.len() as u32;
    clips.push(clip);
}

/// Undo/redo entries and checkpoints hold full clip snapshots of the source
/// as it was. After an append the source is longer, so each snapshot gets the
/// same new span — otherwise an undo or restore would leave the appended
/// footage outside the timeline's clips.
fn extend_history(entry: &mut super::ProjectState, span: &Clip) {
    for journal in entry.undo.iter_mut().chain(entry.redo.iter_mut()) {
        for op in [&mut journal.inverse, &mut journal.redo] {
            if let EditOp::SetClips { clips, .. } = op {
                push_span(clips, span);
            }
        }
    }
    if let Some(project) = entry.project.as_mut() {
        for checkpoint in &mut project.checkpoints {
            push_span(&mut checkpoint.clips, span);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{extend_history, same_source, source_identity, validate_project_source};
    use crate::engine::{JournalEntry, ProjectState};
    use crate::model::*;

    fn media(duration: f64, width: u32) -> Media {
        Media {
            id: uuid::Uuid::new_v4(),
            file_path: "/x.mp4".into(),
            duration,
            frame_rate: 30.0,
            average_frame_rate: None,
            width,
            height: 1080,
            audio_sample_rate: 48000,
            source_audio_channels: None,
            source_audio_tracks: None,
            audio_streams: vec![],
            source_timecode: None,
            codec: "h264".into(),
            imported_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn same_source_needs_matching_duration_size_and_rate() {
        assert!(same_source(&media(120.0, 1920), &media(120.02, 1920)));
        assert!(!same_source(&media(120.0, 1920), &media(95.0, 1920)));
        assert!(!same_source(&media(120.0, 1920), &media(120.0, 3840)));
    }

    #[test]
    fn older_project_fingerprint_accepts_only_unchanged_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.mp4");
        std::fs::write(&path, b"original source").unwrap();
        let mut source = media(120.0, 1920);
        source.file_path = path.to_str().unwrap().to_owned();
        let mut project = Project::new("Older project");
        project.media_fingerprint = Some(source_identity(&source.file_path).unwrap()
            .fingerprint_for("metadata-v1:").unwrap());
        project.media = Some(source);
        validate_project_source(&project).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert_eq!(validate_project_source(&project).unwrap_err().code(), "unavailable");
    }

    #[test]
    fn append_extends_every_snapshot_to_cover_the_new_footage() {
        let mut timeline = Timeline::new(100.0);
        timeline.set_range_included(10.0, 20.0, false, ClipOrigin::Manual);
        let before = timeline.clips.clone();
        let mut project = Project::new("p");
        project.timeline = timeline.clone();
        project.checkpoints.push(Checkpoint {
            id: uuid::Uuid::new_v4(),
            name: "cp".into(),
            created_at: chrono::Utc::now(),
            clips: before.clone(),
            global_padding: Padding::default(),
            cut_count: 1,
            auto: false,
        });
        let snapshot = |clips: Vec<Clip>| EditOp::SetClips { clips, global_padding: Padding::default() };
        let mut entry = ProjectState {
            project: Some(project),
            transcript: None,
            undo: vec![JournalEntry {
                action: EditAction::new(ActionSource::Ui, EditOp::CutRange { start: 10.0, end: 20.0 }),
                inverse: snapshot(Timeline::new(100.0).clips),
                redo: snapshot(before),
            }],
            redo: vec![],
        };
        let span = Clip {
            id: uuid::Uuid::new_v4(),
            source_in: 100.0,
            source_out: 160.0,
            included: true,
            order: 0,
            origin: ClipOrigin::Manual,
            linked_segment_ids: vec![],
        };
        extend_history(&mut entry, &span);

        // Restoring any snapshot over the 160s source keeps the appended
        // minute and still partitions the whole source.
        let covered = |clips: &[Clip]| {
            let mut t = Timeline::new(160.0);
            t.clips = clips.to_vec();
            t.normalize();
            assert_eq!(t.clips.last().unwrap().source_out, 160.0);
            assert!(t.clips.windows(2).all(|w| (w[0].source_out - w[1].source_in).abs() < 1e-9));
            t.included_duration()
        };
        let EditOp::SetClips { clips, .. } = &entry.undo[0].inverse else { unreachable!() };
        assert_eq!(covered(clips), 160.0, "undo to the original keeps the new footage");
        let EditOp::SetClips { clips, .. } = &entry.undo[0].redo else { unreachable!() };
        assert_eq!(covered(clips), 150.0, "the cut survives, plus the new footage");
        let cp = &entry.project.as_ref().unwrap().checkpoints[0];
        assert_eq!(covered(&cp.clips), 150.0);
    }
}
