use crate::engine::Editor;
use crate::error::Result;
use crate::tools;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone)]
struct McpState {
    editor: Editor,
    token: String,
    host: String,
}

#[derive(Clone, serde::Serialize)]
pub struct EndpointInfo {
    pub url: String,
    pub token: String,
}

/// Start the MCP server on a localhost-only port. Persists the per-launch
/// token and the current endpoint to `<data dir>/mcp.json` so the stdio shim
/// can find the running app.
pub async fn start(editor: Editor) -> Result<EndpointInfo> {
    start_with(editor, Some(&crate::store::data_dir().join("mcp.json"))).await
}

/// [`start`] with the discovery file chosen by the caller: `None` writes no
/// file. Tests use that, so a test server never overwrites the real app's
/// `mcp.json` (which would leave the shim pointing at a dead endpoint).
pub async fn start_with(editor: Editor, discovery_file: Option<&std::path::Path>) -> Result<EndpointInfo> {
    let token: String =
        rand::thread_rng().sample_iter(&Alphanumeric).take(40).map(char::from).collect();

    // Stable port across launches: reuse the persisted one when free, fall
    // back to a fresh ephemeral port only if something else grabbed it.
    let preferred: Option<u16> =
        editor.store().get_kv("mcp_port")?.and_then(|p| p.parse().ok());
    let listener = match preferred {
        Some(p) => match tokio::net::TcpListener::bind(("127.0.0.1", p)).await {
            Ok(l) => l,
            Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0").await?,
        },
        None => tokio::net::TcpListener::bind("127.0.0.1:0").await?,
    };
    let port = listener.local_addr()?.port();
    editor.store().set_kv("mcp_port", &port.to_string())?;

    let host = format!("127.0.0.1:{port}");
    let url = format!("http://{host}/mcp");
    let state = McpState { editor, token: token.clone(), host };
    let app = Router::new()
        .route("/mcp", post(handle))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .with_state(state);

    let info = EndpointInfo { url: url.clone(), token: token.clone() };
    if let Some(endpoint_file) = discovery_file {
        write_discovery(endpoint_file, &info)?;
    }

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            crate::applog::write(
                crate::applog::Level::Error,
                crate::applog::Event::McpFailed { error: crate::applog::ErrText::new(&format!("mcp server stopped: {e}")) },
            );
        }
    });
    Ok(info)
}

fn write_discovery(path: &Path, info: &EndpointInfo) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        crate::error::CoreError::InvalidArg("MCP discovery file needs a private directory".into())
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let created = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent);
        created?;
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(crate::error::CoreError::Unavailable(
                "MCP discovery directory must have permissions 0700".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    return Err(crate::error::CoreError::Unavailable(
        "Private MCP discovery is currently supported on Unix only".into(),
    ));

    #[cfg(unix)]
    {
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(file.as_file_mut(), info)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").map(|t| t == token).unwrap_or(false))
        .unwrap_or(false)
}

async fn handle(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if headers.contains_key("origin")
        || headers.get("host").and_then(|value| value.to_str().ok()) != Some(state.host.as_str())
    {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "native loopback clients only" })));
    }
    if !authorized(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "jsonrpc": "2.0", "id": req.get("id"),
                "error": { "code": -32001, "message": "invalid or missing auth token" } })),
        );
    }
    let id = req.get("id").cloned();
    let method = req["method"].as_str().unwrap_or_default().to_string();

    // Notifications need no response body.
    if id.is_none() {
        return (StatusCode::ACCEPTED, Json(json!({})));
    }

    let result: std::result::Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => {
            let requested =
                req["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION).to_string();
            Ok(json!({
                "protocolVersion": requested,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "roughcut", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Local-first AI video editor. YOU are the orchestrator: list_projects to find work, open_project to bring one up in the app (do this when the user asks to open or switch to a project, so they see what you work on), then read_transcript (paged) to understand the footage — read it yourself rather than calling outline_transcript, which runs the app's own (slow, usually on-device) model pass and is meant for the in-app agent. To LOCATE a moment, use find_segments (hybrid BM25 + local-embedding search) — don't export the transcript and index it yourself. To CUT cleanly, pass snap=\"word\"|\"sentence\" to cut_range/apply_edits so cuts land on word/sentence boundaries — don't parse word timestamps yourself. Land cuts/trims/padding with apply_edits in one batch, then ALWAYS call review_flow to catch seams that read broken (mid-sentence cuts, orphaned connectives) and restore what it flags. Everything is non-destructive and undoable; export hands off to an NLE."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            // External clients are orchestrators themselves: granular tools +
            // the apply_edits batch, no meta-tools (see tools::Caller::Mcp).
            let defs: Vec<Value> = tools::defs(tools::Caller::Mcp)
                .into_iter()
                .map(|d| {
                    json!({ "name": d.name, "description": d.description, "inputSchema": d.input_schema })
                })
                .collect();
            Ok(json!({ "tools": defs }))
        }
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or_default().to_string();
            let args = req["params"]["arguments"].clone();
            match tools::dispatch(&state.editor, &name, &args, tools::Caller::Mcp).await {
                Ok(v) => Ok(json!({
                    "content": [{ "type": "text", "text": serde_json::to_string(&v).unwrap_or_default() }],
                    "isError": false
                })),
                Err(e) => Ok(json!({
                    "content": [{ "type": "text", "text": e.to_json().to_string() }],
                    "isError": true
                })),
            }
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };

    let body = match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    };
    (StatusCode::OK, Json(body))
}
