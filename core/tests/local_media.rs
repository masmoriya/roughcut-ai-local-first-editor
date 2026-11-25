use roughcut_core::{events::NullSink, store::SqliteStore, tools, Editor};
use serde_json::{json, Value};
use std::sync::Arc;

async fn call(editor: &Editor, name: &str, args: Value) -> Value {
    tools::dispatch(editor, name, &args, tools::Caller::Ui)
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[tokio::test]
#[ignore = "requires a local video and its matching word-timed transcript"]
async fn transcript_edit_on_local_media() {
    let video = std::env::var("ROUGHCUT_TEST_VIDEO").unwrap();
    let transcript = std::env::var("ROUGHCUT_TEST_TRANSCRIPT").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&directory.path().join("library.sqlite")).unwrap();
    let editor = Editor::bootstrap_with_store(Box::new(store), Arc::new(NullSink)).unwrap();
    let project = call(
        &editor,
        "create_project",
        json!({
            "name": "Local transcript edit verification",
            "file_path": video,
        }),
    )
    .await;
    let pid = &project["id"];
    call(
        &editor,
        "import_timed_transcript",
        json!({
            "project_id": pid,
            "text": std::fs::read_to_string(transcript).unwrap(),
            "confirm_source_clock": true,
            "confirmed_source_path": video,
            "confirmed_media_id": project["media"]["id"],
        }),
    )
    .await;
    let imported = call(&editor, "get_transcript", json!({"project_id": pid})).await;
    let segment = &imported["segments"][0];
    assert!(segment["words"]
        .as_array()
        .is_some_and(|words| !words.is_empty()));
    let cut = call(
        &editor,
        "cut_by_transcript",
        json!({
            "project_id": pid, "segment_ids": [segment["id"]],
        }),
    )
    .await;
    assert_eq!(cut["timeline"]["cut_count"], 1);
    let xml = directory.path().join("cut.xml");
    call(
        &editor,
        "export",
        json!({
            "project_id": pid, "target": "resolve_xml", "out_path": xml,
        }),
    )
    .await;
    assert!(std::fs::read_to_string(xml).unwrap().contains("<xmeml"));
    call(&editor, "undo", json!({"project_id": pid})).await;
    let timeline = call(&editor, "get_timeline", json!({"project_id": pid})).await;
    assert_eq!(timeline["cut_count"], 0);
}
