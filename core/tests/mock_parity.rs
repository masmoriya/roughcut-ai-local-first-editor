//! Drift guard: every tool in the Rust registry must exist in the browser
//! mock — the mock is hand-written, and a missing tool fails silently
//! (browser flows just break). This is a source-text presence check, not a
//! behavioural one; semantics are covered by the Playwright flows.

use roughcut_core::tools;

// These editor commands are exposed only by the desktop Tauri shell; the
// browser mock is not used by the local Resolve workflow.
const DESKTOP_ONLY_TOOLS: &[&str] = &[
    "highlight_source_ranges",
    "import_timed_transcript",
    "protect_source_ranges",
    "cancel_transcription",
    "review_search_cuts",
    "correct_transcript_segment",
    "get_history_cut_review",
];

#[test]
fn browser_mock_covers_every_registry_tool() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/src/ipc/mockApi.ts");
    let src = std::fs::read_to_string(&path).expect("read frontend/src/ipc/mockApi.ts");
    let missing: Vec<String> = tools::defs(tools::Caller::Ui)
        .into_iter()
        .map(|d| d.name)
        .filter(|name| !DESKTOP_ONLY_TOOLS.contains(&name.as_str()))
        .filter(|name| {
            // Entries are object-method shorthand in the `tools` table:
            //   list_projects(): ... | create_project(args): ... | async foo(args)
            !src.lines().map(str::trim_start).any(|l| {
                l.starts_with(&format!("{name}(")) || l.starts_with(&format!("async {name}("))
            })
        })
        .map(|n| n.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "browser mock (frontend/src/ipc/mockApi.ts) is missing registry tools: {missing:?}"
    );
}
