//! Tauri shell: registers the IPC commands (one generic `call_tool` plus a
//! couple of conveniences), forwards core events to the webview, serves the
//! preview's media files (`media`, the `rcmedia://` protocol).

mod media;
mod media_access;
mod multicam;

use roughcut_core::applog;
use roughcut_core::events::EventSink;
use roughcut_core::{tools, Editor};
use serde_json::Value;
use tauri::{Emitter, Manager, State};

struct TauriSink(tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.0.emit(event, payload);
    }
}

struct AppState {
    editor: Editor,
    media_access: std::sync::Arc<media_access::MediaAccess>,
    revealed_exports: std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, std::path::PathBuf>>,
}

#[tauri::command]
async fn call_tool(
    state: State<'_, AppState>,
    name: String,
    mut args: Value,
) -> Result<Value, Value> {
    if !desktop_tool_enabled(&name) {
        return Err(serde_json::json!({
            "code": "unavailable",
            "message": "This command is unavailable in the desktop editor"
        }));
    }
    if name == "import_media" && args["project_id"].as_str().is_none() {
        return Err(serde_json::json!({
            "code": "invalid_argument",
            "message": "Choose a project before importing media"
        }));
    }
    if let Some(args) = args.as_object_mut() {
        match name.as_str() {
            "remove_silences" => { args.insert("review".into(), Value::Bool(true)); }
            "generate_rough_cut" => { args.insert("apply".into(), Value::Bool(false)); }
            _ => {}
        }
    }
    let result = tools::dispatch(&state.editor, &name, &args, tools::Caller::Ui)
        .await
        .map_err(|e| e.to_json())?;
    state.media_access.record(&name, &result);
    if matches!(name.as_str(), "export" | "export_clip" | "export_clips") {
        let mut exports = state.revealed_exports.lock().unwrap();
        for path in result["path"].as_str().into_iter().chain(
            result["results"].as_array().into_iter().flatten().filter_map(|entry| entry["path"].as_str()),
        ) {
            let path = std::path::Path::new(path);
            if let Ok(canonical) = path.canonicalize() {
                exports.insert(path.to_owned(), canonical);
            }
        }
    }
    Ok(result)
}

#[tauri::command]
fn list_tools() -> Vec<tools::ToolDef> {
    tools::defs(tools::Caller::Ui)
        .into_iter()
        .filter(|tool| desktop_tool_enabled(&tool.name))
        .collect()
}

fn desktop_tool_enabled(name: &str) -> bool {
    matches!(name,
        "list_projects" | "open_project" | "create_project" | "duplicate_project"
        | "save_project" | "delete_project" | "restore_project"
        | "get_timeline" | "get_transcript" | "get_media_assets" | "import_media"
        | "append_media" | "attach_screen_media" | "set_layout" | "set_audio_offset"
        | "transcribe" | "import_timed_transcript" | "correct_transcript_segment" | "read_transcript"
        | "review_search_cuts" | "highlight_source_ranges" | "protect_source_ranges"
        | "detect_silences" | "remove_silences" | "detect_fillers" | "detect_takes"
        | "generate_rough_cut" | "get_cut_proposal" | "apply_cut_proposal"
        | "discard_cut_proposal" | "skip_proposed_cuts" | "get_suggestions"
        | "accept_suggestion" | "accept_all_suggestions" | "dismiss_suggestion"
        | "cut_range" | "cut_passage" | "restore_range" | "cut_by_transcript"
        | "restore_by_transcript" | "trim_clip" | "split_clip" | "set_global_padding"
        | "apply_edits" | "undo_actions" | "undo" | "redo" | "get_history" | "get_history_cut_review" | "jump_to"
        | "create_checkpoint" | "get_checkpoints" | "restore_checkpoint" | "delete_checkpoint"
        | "get_clip_candidates" | "frame_candidate" | "create_clip" | "list_clips"
        | "update_clip" | "begin_clip_edit" | "end_clip_edit" | "delete_clip"
        | "generate_captions" | "export" | "export_clip" | "export_clips"
        | "get_preferences" | "set_preferences"
    )
}

#[cfg(test)]
#[test]
fn manual_editor_surface() {
    for command in [
        "transcribe", "import_timed_transcript", "review_search_cuts", "apply_cut_proposal", "get_history_cut_review",
        "undo", "export", "export_clip", "export_clips",
    ] {
        assert!(desktop_tool_enabled(command), "{command}");
    }
    for command in ["retime_words", "apply_instruction", "find_clips", "unlisted_command"] {
        assert!(!desktop_tool_enabled(command), "{command}");
    }
}

/// The app log's file, for Setup's "Reveal logs" (None if it couldn't be
/// opened). Outside the tool registry, like `reveal_path`: it reads a local
/// file path, not the project.
#[tauri::command]
fn log_path() -> Option<String> {
    applog::log_file().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn reveal_path(state: State<'_, AppState>, path: String) -> Result<(), Value> {
    #[cfg(target_os = "macos")]
    {
        let requested = std::path::Path::new(&path);
        let is_log = applog::log_file().as_deref() == Some(requested);
        let expected = state.revealed_exports.lock().unwrap().get(requested).cloned();
        if !is_log && expected.is_none() {
            return Err(serde_json::json!({ "error": { "code": "unavailable", "message": "Only this session's exports and the app log can be revealed" } }));
        }
        if !requested.symlink_metadata().is_ok_and(|metadata| metadata.is_file()) {
            return Err(serde_json::json!({ "error": { "code": "unavailable", "message": "The file to reveal is missing or has been replaced" } }));
        }
        let path = requested.canonicalize()
            .map_err(|e| serde_json::json!({ "error": { "code": "io", "message": e.to_string() } }))?;
        if expected.is_some_and(|approved| approved != path) {
            return Err(serde_json::json!({ "error": { "code": "unavailable", "message": "The export path changed after the file was saved" } }));
        }
        let status = tauri::async_runtime::spawn_blocking(move || {
            std::process::Command::new("/usr/bin/open").arg("-R").arg(path).status()
        }).await
            .map_err(|e| serde_json::json!({ "error": { "code": "io", "message": e.to_string() } }))?
            .map_err(|e| serde_json::json!({ "error": { "code": "io", "message": e.to_string() } }))?;
        if !status.success() {
            return Err(serde_json::json!({ "error": { "code": "io", "message": "Finder could not reveal the file" } }));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, path);
        Err(serde_json::json!({ "error": { "code": "unavailable", "message": "Open the saved path in your file manager" } }))
    }
}


#[tauri::command]
async fn install_transcription_model(app: tauri::AppHandle) -> Result<(), Value> {
    static INSTALL: tauri::async_runtime::Mutex<()> = tauri::async_runtime::Mutex::const_new(());
    let _install = INSTALL.lock().await;
    let sink: roughcut_core::events::SharedSink = std::sync::Arc::new(TauriSink(app));
    roughcut_core::setup::download_whisper_model(&sink, "compact")
        .await
        .map(|_| ())
        .map_err(|error| error.to_json())
}

#[tauri::command]
fn setup_status() -> Value {
    use roughcut_core::adapters::video::{ffmpeg_path, ffprobe_path};
    let ffmpeg = ffmpeg_path();
    let ffprobe = ffprobe_path();
    serde_json::json!({
        "transcription_model_installed": roughcut_core::adapters::transcribe::resolve_whisper_model().is_some(),
        "ffmpeg": ffmpeg.is_some() && ffprobe.is_some(),
        "ffmpeg_path": ffmpeg.as_deref().map(|path| path.to_string_lossy()),
        "ffprobe_path": ffprobe.as_deref().map(|path| path.to_string_lossy()),
        "resolve_installed": cfg!(target_os = "macos").then(roughcut_core::resolve::app_installed),
    })
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be first so a second launch focuses the existing window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri::plugin::Builder::<tauri::Wry, ()>::new("local-navigation")
            .on_navigation(|_, url| {
                if !url.username().is_empty() || url.password().is_some() {
                    return false;
                }
                match (url.scheme(), url.host_str(), url.port()) {
                    ("tauri", Some("localhost"), None)
                    | ("http" | "https", Some("tauri.localhost"), None) => true,
                    ("http", Some("127.0.0.1"), Some(1420)) => cfg!(debug_assertions),
                    _ => url.as_str() == "about:blank",
                }
            }).build())
        // The preview's <video>/<audio>: full byte ranges, off the main thread.
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, |ctx, request, responder| {
            media::handle(ctx, request, responder)
        })
        .setup(|app| {
            // The app log first, so a failed start is in it.
            applog::init(&applog::log_dir());
            let sink = std::sync::Arc::new(TauriSink(app.handle().clone()));
            let editor = match Editor::bootstrap(sink) {
                Ok(editor) => editor,
                Err(e) => {
                    applog::write(applog::Level::Error, applog::Event::AppStartFailed { code: e.code(), error: (&e).into() });
                    return Err(e.into());
                }
            };
            applog::app_start(&editor, env!("CARGO_PKG_VERSION"));
            app.manage(AppState { editor, media_access: Default::default(), revealed_exports: Default::default() });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            call_tool,
            list_tools,
            reveal_path,
            log_path,
            setup_status,
            install_transcription_model,
            multicam::open_resolve_review,
            multicam::resolve_cut_plan,
        ])
        .build(tauri::generate_context!())
        .expect("error while building RoughCut")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                applog::write(applog::Level::Info, applog::Event::AppExit {});
            }
        });
}
