//! Persistence for offline Resolve review. Resolve remains the edit target.
use super::{media::validate_project_source, Editor};
use crate::error::{CoreError, Result};
use crate::model::{Project, Timeline};
use crate::multicam::{ResolveCutPlan, ResolveTimelineBinding};
use uuid::Uuid;

impl Editor {
    pub async fn create_resolve_review(
        &self,
        binding: ResolveTimelineBinding,
        file_path: &str,
    ) -> Result<Project> {
        binding.validate()?;
        let (media, identity) = self.probe_attached_media(file_path).await?;
        let identity = identity
            .ok_or_else(|| CoreError::Unavailable("Choose a local playback export".into()))?;
        let fingerprint = identity.fingerprint()?;
        let digest = tokio::task::spawn_blocking(move || identity.content_sha256())
            .await
            .map_err(|_| CoreError::Other("Playback verification was interrupted".into()))??;
        binding.validate_playback(&media, &digest)?;
        let mut project = Project::new(&binding.timeline.timeline_name);
        project.timeline = Timeline::new(media.duration);
        project.media = Some(media);
        project.media_fingerprint = Some(fingerprint);
        project.resolve_timeline = Some(binding);
        validate_project_source(&project)?;
        self.insert_project(project)
    }

    pub fn resolve_cut_plan(&self, project_id: Uuid) -> Result<ResolveCutPlan> {
        let (project, transcript) = self.snapshot(project_id)?;
        let binding = project
            .resolve_timeline
            .ok_or_else(|| CoreError::NotFound("Resolve timeline binding".into()))?;
        if transcript.is_none() || project.clip_session.is_some() || project.cut_proposal.is_some()
        {
            return Err(CoreError::InvalidArg(
                "Import the matching transcript and finish any open review before copying the plan"
                    .into(),
            ));
        }
        // This is a review artifact. A Resolve writer must compare against a
        // newly captured Resolve snapshot before trusting it for any mutation.
        binding.plan(&binding.timeline, &project.timeline)
    }

    pub(super) fn require_single_file_project(&self, project_id: Uuid) -> Result<()> {
        self.with_project(project_id, |project, _| {
            if project.resolve_timeline.is_some() {
                return Err(CoreError::InvalidArg(
                    "The Resolve review uses its bound playback export".into(),
                ));
            }
            Ok(())
        })
    }
}
