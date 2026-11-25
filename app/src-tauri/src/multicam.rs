use crate::AppState;
use roughcut_core::model::Project;
use roughcut_core::multicam::{ResolveCutPlan, ResolveTimelineBinding};
use roughcut_core::CoreError;
use std::io::Read;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const MAX_BINDING_BYTES: u64 = 16 * 1024;

#[tauri::command]
pub async fn open_resolve_review(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Project>, serde_json::Value> {
    let selection = tauri::async_runtime::spawn_blocking(move || {
        let Some(binding) = app
            .dialog()
            .file()
            .add_filter("Resolve binding", &["json"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let binding = binding
            .into_path()
            .map_err(|_| CoreError::InvalidArg("Choose a local Resolve binding".into()))?;
        if !binding.metadata()?.is_file() {
            return Err(CoreError::InvalidArg(
                "Choose a regular Resolve binding file".into(),
            ));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(binding)?
            .take(MAX_BINDING_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BINDING_BYTES {
            return Err(CoreError::InvalidArg(
                "Choose a Resolve binding no larger than 16 KiB".into(),
            ));
        }
        let binding: ResolveTimelineBinding = serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::InvalidArg("Invalid Resolve timeline binding JSON".into()))?;
        binding.validate()?;
        let Some(playback) = app
            .dialog()
            .file()
            .add_filter("Playback export", &["mov", "mp4", "mkv"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let playback = playback
            .into_path()
            .map_err(|_| CoreError::InvalidArg("Choose a local playback export".into()))?;
        Ok(Some((binding, playback)))
    })
    .await
    .map_err(|_| CoreError::Other("File selection was interrupted".into()).to_json())?
    .map_err(|error: CoreError| error.to_json())?;
    let Some((binding, playback)) = selection else {
        return Ok(None);
    };
    let path = playback.to_str().ok_or_else(|| {
        CoreError::InvalidArg("Choose a playback filename in UTF-8".into()).to_json()
    })?;
    let project = state
        .editor
        .create_resolve_review(binding, path)
        .await
        .map_err(|error| error.to_json())?;
    if let Ok(value) = serde_json::to_value(&project) {
        state.media_access.record("create_project", &value);
    }
    Ok(Some(project))
}

#[tauri::command]
pub fn resolve_cut_plan(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> Result<ResolveCutPlan, serde_json::Value> {
    state
        .editor
        .resolve_cut_plan(project_id)
        .map_err(|error| error.to_json())
}
