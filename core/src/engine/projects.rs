//! Project lifecycle (create / duplicate / open / save / list / delete) and
//! the read accessors built on [`Editor::with_project`].

use super::{Editor, ProjectState};
use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent};
use crate::model::*;
use chrono::Utc;
use uuid::Uuid;

fn normalize_phrase_list(entries: &mut Vec<String>) -> Result<()> {
    if entries.len() > 200 || entries.iter().map(|entry| entry.chars().count()).sum::<usize>() > 4000
        || entries.iter().any(|entry| entry.chars().any(char::is_control))
    {
        return Err(CoreError::InvalidArg("Phrase lists support up to 200 phrases and 4,000 characters, with one phrase per line".into()));
    }
    let mut seen = std::collections::HashSet::new();
    entries.retain_mut(|entry| {
        *entry = entry.trim().to_owned();
        !entry.is_empty() && seen.insert(entry.clone())
    });
    Ok(())
}

impl Editor {
    pub async fn create_project(&self, name: &str, file_path: Option<&str>) -> Result<Project> {
        let mut project = Project::new(name);
        if let Some(path) = file_path {
            let (media, identity) = self.probe_attached_media(path).await?;
            project.media_fingerprint = identity.as_ref().map(super::media::SourceIdentity::fingerprint).transpose()?;
            project.timeline = Timeline::new(media.duration);
            project.media = Some(media);
        }
        self.insert_project(project)
    }

    pub(super) fn insert_project(&self, project: Project) -> Result<Project> {
        self.inner.store.save_project(&project)?;
        {
            let mut state = self.inner.state.lock().unwrap();
            state.insert(
                project.id,
                ProjectState { project: Some(project.clone()), ..Default::default() },
            );
        }
        send(&self.inner.sink, CoreEvent::ProjectsChanged {});
        Ok(project)
    }

    /// Save-as: clone media + current cut state under a new name with a fresh
    /// undo history. The original project is untouched.
    pub fn duplicate_project(&self, project_id: Uuid, name: &str) -> Result<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::InvalidArg("Enter a name for the project copy".into()));
        }
        self.ensure_loaded(project_id)?;
        let (mut copy, transcript) = {
            let state = self.inner.state.lock().unwrap();
            let entry =
                state.get(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let source = entry.project.as_ref().ok_or_else(|| CoreError::NotFound("project".into()))?;
            if source.deleted_at.is_some() {
                return Err(CoreError::InvalidArg("Restore the project from trash before duplicating it".into()));
            }
            if source.clip_session.is_some() {
                return Err(CoreError::InvalidArg("Save or cancel the clip edit before duplicating the project".into()));
            }
            (source.clone(), entry.transcript.clone())
        };
        copy.id = Uuid::new_v4();
        copy.name = name.to_string();
        copy.created_at = Utc::now();
        copy.updated_at = Utc::now();
        self.inner.store.save_edit(&copy, transcript.as_ref(), r#"{"undo":[],"redo":[]}"#, None)?;
        {
            let mut state = self.inner.state.lock().unwrap();
            state.insert(
                copy.id,
                ProjectState {
                    project: Some(copy.clone()),
                    transcript,
                    ..Default::default()
                },
            );
        }
        send(&self.inner.sink, CoreEvent::ProjectsChanged {});
        Ok(copy)
    }

    pub fn open_project(&self, project_id: Uuid) -> Result<Project> {
        self.ensure_loaded(project_id)?;
        // Old projects (transcribed before indexing existed) gain a semantic
        // index in the background the first time they're opened.
        if self.inference_enabled() {
            self.backfill_index_if_missing(project_id);
        }
        self.with_project(project_id, |p, _| Ok(p.clone()))
    }

    pub fn save_project(&self, project_id: Uuid) -> Result<Project> {
        self.ensure_loaded(project_id)?;
        let state = self.inner.state.lock().unwrap();
        let entry = state.get(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
        let project = entry.project.clone().ok_or_else(|| CoreError::NotFound("project".into()))?;
        self.inner.store.save_project(&project)?;
        if let Some(t) = &entry.transcript {
            self.inner.store.save_transcript(project_id, t)?;
        }
        Ok(project)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        self.inner.store.list_projects()
    }

    /// Move a project to the TRASH (soft delete): edit state survives and
    /// [`Editor::restore_project`] brings it back; the startup sweep purges
    /// trash older than 30 days. The source video file is never touched.
    pub fn delete_project(&self, project_id: Uuid) -> Result<()> {
        self.ensure_loaded(project_id)?;
        {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let project =
                entry.project.as_mut().ok_or_else(|| CoreError::NotFound("project".into()))?;
            project.deleted_at = Some(Utc::now());
            project.updated_at = Utc::now();
            self.inner.store.save_project(project)?;
        }
        self.inner.state.lock().unwrap().remove(&project_id);
        send(&self.inner.sink, CoreEvent::ProjectsChanged {});
        Ok(())
    }

    /// Bring a trashed project back, edit state and all.
    pub fn restore_project(&self, project_id: Uuid) -> Result<Project> {
        self.ensure_loaded(project_id)?;
        let project = {
            let mut state = self.inner.state.lock().unwrap();
            let entry =
                state.get_mut(&project_id).ok_or_else(|| CoreError::NotFound("project".into()))?;
            let project =
                entry.project.as_mut().ok_or_else(|| CoreError::NotFound("project".into()))?;
            project.deleted_at = None;
            project.updated_at = Utc::now();
            self.inner.store.save_project(project)?;
            project.clone()
        };
        send(&self.inner.sink, CoreEvent::ProjectsChanged {});
        Ok(project)
    }

    /// Hard-delete trash older than `days` (called once at startup).
    pub fn purge_trash(&self, days: i64) -> Result<u32> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let mut purged = 0;
        for p in self.inner.store.list_projects()? {
            if matches!(p.deleted_at, Some(t) if t < cutoff) {
                self.inner.store.delete_project(p.id)?;
                self.inner.state.lock().unwrap().remove(&p.id);
                purged += 1;
            }
        }
        Ok(purged)
    }

    pub fn get_timeline(&self, project_id: Uuid) -> Result<Timeline> {
        self.with_project(project_id, |p, _| Ok(p.timeline.clone()))
    }

    /// Waveform peaks + thumbnails for the project's media (generated on
    /// first call, cached after). Returns file PATHS for the asset protocol.
    /// A missing playable copy (sources WebKit can't stream) builds in the
    /// background, started BEFORE the peaks and thumbnails so the preview
    /// doesn't wait on them; `media-assets-changed` fires when it lands or
    /// fails (then `playback_error` says why, and it isn't retried this run).
    pub async fn media_assets(
        &self,
        project_id: Uuid,
    ) -> Result<crate::adapters::video::MediaAssets> {
        use crate::adapters::video::{ensure_playable_stream, playable_stream_failure, playable_stream_path};
        let (media, stream) = self.with_project(project_id, |p, transcript| {
            let media = p.media.clone().ok_or_else(|| CoreError::InvalidArg("project has no media".into()))?;
            super::media::validate_project_source(p)?;
            let stream = transcript.and_then(|value| value.source_audio_stream_index)
                .or_else(|| p.resolve_timeline.as_ref().map(|binding| binding.dialogue_stream_index))
                .filter(|_| media.source_audio_tracks.is_some_and(|tracks| tracks > 1));
            Ok((media, stream))
        })?;
        // Only when there's neither a copy nor a recorded failure: otherwise
        // the event below would trigger a refetch that lands here again.
        if !self.demo_mode()
            && playable_stream_path(&media, stream).is_none()
            && playable_stream_failure(&media, stream).is_none()
        {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let editor = self.clone();
                let media = media.clone();
                handle.spawn(async move {
                    match ensure_playable_stream(&media, stream).await {
                        Ok(Some(_)) | Err(_) => {
                            send(&editor.inner.sink, CoreEvent::MediaAssetsChanged { project_id })
                        }
                        Ok(None) => {} // plays as-is, or another call is building it
                    }
                });
            }
        }
        self.inner.video.media_assets(&media, stream).await
    }

    pub fn get_transcript(&self, project_id: Uuid) -> Result<Option<Transcript>> {
        self.with_project(project_id, |project, transcript| {
            if let Some(transcript) = transcript {
                super::media::validate_transcript_source(transcript, project.media.as_ref())?;
            }
            Ok(transcript.cloned())
        })
    }

    /// Capture matching project and transcript revisions under one state lock.
    pub fn snapshot(&self, project_id: Uuid) -> Result<(Project, Option<Transcript>)> {
        self.with_project(project_id, |project, transcript| {
            super::media::validate_project_source(project)?;
            if let Some(transcript) = transcript {
                super::media::validate_transcript_source(transcript, project.media.as_ref())?;
            }
            Ok((project.clone(), transcript.cloned()))
        })
    }

    pub fn get_preferences(&self) -> Result<Preferences> {
        self.inner.store.load_preferences()
    }

    pub fn set_preferences(&self, prefs: Preferences) -> Result<Preferences> {
        let _write = self.inner.prefs_write.lock().unwrap();
        self.inner.store.save_preferences(&prefs)?;
        Ok(prefs)
    }

    /// Apply a PARTIAL preferences update: only the keys given change, nested
    /// objects (`default_padding`) merge, unknown keys are rejected rather than
    /// silently dropped. Zero egress: only the user may point inference at a
    /// host off this machine — post-transcribe cleanup and the embedding index
    /// send transcript text wherever `inference_endpoint` names.
    pub fn update_preferences(
        &self,
        patch: &serde_json::Value,
        source: ActionSource,
    ) -> Result<Preferences> {
        self.update_preferences_checked(patch, source, None)
    }

    pub(crate) fn update_preferences_checked(
        &self,
        patch: &serde_json::Value,
        source: ActionSource,
        expected_rule_sets: Option<&serde_json::Value>,
    ) -> Result<Preferences> {
        let patch = patch
            .as_object()
            .ok_or_else(|| CoreError::InvalidArg("preferences must be an object".into()))?;
        if !self.inference_enabled() && ["inference_endpoint", "inference_model", "embedding_model", "model_tier"]
            .iter().any(|key| patch.contains_key(*key))
        {
            return Err(CoreError::Unavailable("Model settings are disabled in this editor".into()));
        }
        let _write = self.inner.prefs_write.lock().unwrap();
        let current = self.get_preferences()?;
        if patch.contains_key("saved_rule_sets")
            && expected_rule_sets != Some(&serde_json::to_value(&current.saved_rule_sets)?)
        {
            return Err(CoreError::Unavailable("Saved rule sets changed. Reload the list and try again.".into()));
        }
        let serde_json::Value::Object(mut merged) = serde_json::to_value(&current)? else {
            return Err(CoreError::Other("preferences did not serialize to an object".into()));
        };
        let unknown: Vec<&str> =
            patch.keys().filter(|k| !merged.contains_key(*k)).map(String::as_str).collect();
        if !unknown.is_empty() {
            let mut known: Vec<&str> = merged.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(CoreError::InvalidArg(format!(
                "unknown preference(s): {}; known: {}",
                unknown.join(", "),
                known.join(", ")
            )));
        }
        merge_into(&mut merged, patch);
        let mut next: Preferences = serde_json::from_value(serde_json::Value::Object(merged))?;
        for (key, entries) in [
            ("custom_filler_words", &mut next.custom_filler_words),
            ("profanity_phrases", &mut next.profanity_phrases),
            ("keyword_phrases", &mut next.keyword_phrases),
            ("never_cut_phrases", &mut next.never_cut_phrases),
        ] {
            if !patch.contains_key(key) { continue; }
            normalize_phrase_list(entries)?;
        }
        if patch.contains_key("saved_rule_sets") {
            if source != ActionSource::Ui || next.saved_rule_sets.len() > 16 {
                return Err(CoreError::InvalidArg("Save up to 16 rule sets in the editor".into()));
            }
            let mut names = std::collections::HashSet::new();
            for set in &mut next.saved_rule_sets {
                set.name = set.name.trim().to_owned();
                if set.name.is_empty() || set.name.chars().count() > 80
                    || set.name.chars().any(char::is_control) || !names.insert(set.name.to_lowercase())
                {
                    return Err(CoreError::InvalidArg("Each rule set needs a unique name of up to 80 characters".into()));
                }
                for list in [&mut set.custom_filler_words, &mut set.profanity_phrases,
                    &mut set.keyword_phrases, &mut set.never_cut_phrases]
                {
                    normalize_phrase_list(list)?;
                }
            }
        }
        // The model switch, its model and the consent are the user's, set in
        // the app (shell commands), never through this tool by any caller.
        if next.model_mode != current.model_mode
            || next.hosted_model != current.hosted_model
            || next.hosted_consent_at != current.hosted_consent_at
        {
            return Err(CoreError::InvalidArg(
                "model_mode, hosted_model and hosted_consent_at can only be changed in the app \
                 (the LOCAL / HOSTED switch and Setup)"
                    .into(),
            ));
        }
        if source != ActionSource::Ui
            && next.inference_endpoint != current.inference_endpoint
            && !is_loopback_url(&next.inference_endpoint)
        {
            return Err(CoreError::InvalidArg(format!(
                "inference_endpoint {} is not on this machine; only the user can send \
                 transcripts off-device (change it in the app's Setup screen)",
                next.inference_endpoint
            )));
        }
        self.inner.store.save_preferences(&next)?;
        Ok(next)
    }

    /// Presentation layout — project metadata, persisted, not journaled.
    pub fn set_layout(&self, project_id: uuid::Uuid, layout: crate::model::Layout) -> crate::error::Result<()> {
        self.ensure_loaded(project_id)?;
        let mut state = self.inner.state.lock().unwrap();
        let entry = state
            .get_mut(&project_id)
            .and_then(|e| e.project.as_mut())
            .ok_or_else(|| crate::error::CoreError::NotFound(format!("project {project_id}")))?;
        entry.layout = layout;
        self.inner.store.save_project(entry)?;
        crate::events::send(
            &self.sink(),
            crate::events::CoreEvent::TimelineChanged { project_id },
        );
        Ok(())
    }

    /// A/V sync nudge — project metadata, persisted, not journaled.
    pub fn set_audio_offset(&self, project_id: uuid::Uuid, offset_s: f64) -> crate::error::Result<()> {
        self.require_single_file_project(project_id)?;
        self.ensure_loaded(project_id)?;
        let mut state = self.inner.state.lock().unwrap();
        let entry = state
            .get_mut(&project_id)
            .and_then(|e| e.project.as_mut())
            .ok_or_else(|| crate::error::CoreError::NotFound(format!("project {project_id}")))?;
        entry.audio_offset_s = offset_s;
        self.inner.store.save_project(entry)?;
        Ok(())
    }
}

/// JSON merge: nested objects merge key by key; anything else replaces.
fn merge_into(
    base: &mut serde_json::Map<String, serde_json::Value>,
    patch: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in patch {
        match (base.get_mut(k), v) {
            (Some(serde_json::Value::Object(b)), serde_json::Value::Object(p)) => merge_into(b, p),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Is `url` served from this machine (localhost, 127.0.0.0/8, ::1)?
fn is_loopback_url(url: &str) -> bool {
    let Some(host) = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned))
    else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_sets_survive_reopen_and_reject_stale_updates() {
        use crate::{events::NullSink, store::SqliteStore};
        use std::sync::Arc;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("library.db");
        let open = || Editor::bootstrap_with_store(
            Box::new(SqliteStore::open(&path).unwrap()),
            Arc::new(NullSink),
        ).unwrap();
        let editor = open();
        let initial = serde_json::json!([]);
        let patch = serde_json::json!({"saved_rule_sets": [{
            "name": " Interview ",
            "custom_filler_words": [" um ", "um"],
            "profanity_phrases": [],
            "keyword_phrases": [],
            "never_cut_phrases": ["you know"],
        }]});
        editor.update_preferences_checked(&patch, ActionSource::Ui, Some(&initial)).unwrap();
        assert!(editor.update_preferences_checked(
            &serde_json::json!({"saved_rule_sets": []}), ActionSource::Ui, Some(&initial),
        ).is_err());
        drop(editor);

        let restored = open().get_preferences().unwrap().saved_rule_sets;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, "Interview");
        assert_eq!(restored[0].custom_filler_words, vec![String::from("um")]);
        assert_eq!(restored[0].never_cut_phrases, vec![String::from("you know")]);
    }

    #[test]
    fn loopback_urls() {
        for local in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:47861/v1",
            "http://127.1.2.3/v1",
            "http://[::1]:8080/v1",
            "http://LOCALHOST/v1",
            "http://llm.localhost/v1",
        ] {
            assert!(is_loopback_url(local), "{local} should be local");
        }
        for remote in [
            "https://api.example.com/v1",
            "http://192.168.1.10:11434/v1",
            "http://0.0.0.0:11434/v1",
            "http://localhost.example.com/v1",
            "not a url",
            "",
        ] {
            assert!(!is_loopback_url(remote), "{remote} should not be local");
        }
    }

    #[test]
    fn merge_is_deep_for_objects_and_replaces_otherwise() {
        let mut base = serde_json::json!({
            "a": 1,
            "pad": { "start_s": 0.1, "end_s": 0.2, "linked": true },
            "words": ["um"]
        });
        let patch = serde_json::json!({ "pad": { "end_s": 0.5 }, "words": ["uh"] });
        merge_into(base.as_object_mut().unwrap(), patch.as_object().unwrap());
        assert_eq!(
            base,
            serde_json::json!({
                "a": 1,
                "pad": { "start_s": 0.1, "end_s": 0.5, "linked": true },
                "words": ["uh"]
            })
        );
    }
}
