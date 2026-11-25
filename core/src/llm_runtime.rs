//! Managed local LLM runtime (spec M4/M7): detect Ollama first; otherwise
//! install and run a llama.cpp `llama-server` sidecar. Everything is
//! user-triggered, with progress events — the same lean-app/download-on-
//! demand story as the whisper models.
//!
//! macOS-focused per project direction; the release-asset naming used here
//! (`llama-<tag>-bin-macos-{arm64,x64}.tar.gz`) matches ggml-org/llama.cpp.

use crate::engine::Editor;
use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent, ProgressTask, SharedSink};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use tokio::process::{Child, Command};

const MANAGED_PORT: u16 = 47861;

fn managed_child() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

fn bin_dir() -> PathBuf {
    crate::store::data_dir().join("bin")
}

fn llama_server_path() -> PathBuf {
    bin_dir().join("llama-server")
}

// Status questions (reported through `setup::status()` — the ONE
// provisioning status the UI fetches).

pub fn ollama_cli_available() -> bool {
    std::process::Command::new("which")
        .arg("ollama")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn llama_server_installed() -> bool {
    llama_server_path().is_file()
}

pub fn managed_running() -> bool {
    // try_wait notices a child that crashed or was killed externally —
    // is_some() alone reported a corpse as "running" forever.
    let mut guard = managed_child().lock().unwrap();
    match guard.as_mut() {
        None => false,
        Some(child) => match child.try_wait() {
            Ok(Some(_exit)) => {
                *guard = None;
                false
            }
            Ok(None) => true,
            Err(_) => false,
        },
    }
}

/// GGUF files available in the models dir (for the sidecar path).
pub fn list_ggufs() -> Vec<String> {
    std::fs::read_dir(crate::adapters::transcribe::models_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "gguf").unwrap_or(false))
                .map(|e| e.path().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// `ollama pull <model>` with progress events (task `model_pull`).
pub async fn ollama_pull(sink: &SharedSink, model: &str) -> Result<()> {
    if !ollama_cli_available() {
        return Err(CoreError::Unavailable("ollama CLI not found".into()));
    }
    let mut child = Command::new("ollama")
        .args(["pull", model])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| CoreError::Unavailable(format!("ollama pull: {e}")))?;
    let model2 = model.to_string();
    let progress_task = child.stderr.take().map(|stderr| {
        crate::progress::stream_progress(
            stderr,
            sink.clone(),
            ProgressTask::ModelPull,
            None,
            crate::progress::Delimiter::CarriageReturn,
            move |text| {
                let i = text.rfind('%')?;
                let p: u8 = text[..i].split_whitespace().last()?.parse().ok()?;
                Some((f64::from(p) / 100.0, format!("pulling {model2}: {p}%")))
            },
        )
    });
    let status = child
        .wait()
        .await
        .map_err(|e| CoreError::Other(format!("ollama pull: {e}")))?;
    if let Some(t) = progress_task {
        let _ = t.await;
    }
    if !status.success() {
        return Err(CoreError::Other(format!("ollama pull {model} failed")));
    }
    send(&sink, CoreEvent::progress(ProgressTask::ModelPull, None, 1.0, format!("{model} ready")));
    Ok(())
}

/// Managed downloads require a reviewed artifact manifest.
pub async fn install_llama_server(_sink: &SharedSink) -> Result<String> {
    Err(CoreError::Unavailable("Verified llama-server installation is unavailable".into()))
}

pub async fn download_gguf(_sink: &SharedSink, _url: &str) -> Result<String> {
    Err(CoreError::Unavailable("Verified GGUF downloads are unavailable".into()))
}

pub async fn start_managed(_editor: &Editor, _gguf_path: &str) -> Result<String> {
    Err(CoreError::Unavailable("Verified managed runtime is unavailable".into()))
}

pub fn stop_managed() {
    if let Some(mut child) = managed_child().lock().unwrap().take() {
        let _ = child.start_kill();
    }
}
