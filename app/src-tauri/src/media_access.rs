use std::collections::HashMap;
use std::fs::{File, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

#[derive(Default)]
pub struct MediaAccess(Mutex<HashMap<PathBuf, ApprovedMedia>>);

#[derive(Clone)]
struct ApprovedMedia {
    canonical: PathBuf,
    bytes: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl ApprovedMedia {
    fn matches(&self, metadata: &Metadata) -> bool {
        if !metadata.is_file() || metadata.len() != self.bytes
            || !metadata.modified().is_ok_and(|modified| modified == self.modified) {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino()) == self.identity
        }
        #[cfg(not(unix))]
        true
    }
}

impl MediaAccess {
    pub fn record(&self, tool: &str, result: &Value) {
        match tool {
            "open_project" | "create_project" | "duplicate_project" | "restore_project"
            | "save_project" => {
                self.add_checked(&result["media"]["file_path"], false, result["media_fingerprint"].as_str());
                self.add_checked(&result["screen_media"]["file_path"], false, result["screen_media_fingerprint"].as_str());
            }
            "attach_screen_media" => {
                if let Some(fingerprint) = result["screen_media_fingerprint"].as_str() {
                    self.add_checked(&result["screen_media"]["file_path"], false, Some(fingerprint));
                }
            }
            "append_media" => {
                if let Some(fingerprint) = result["media_fingerprint"].as_str() {
                    self.add_checked(&result["media"]["file_path"], false, Some(fingerprint));
                }
            }
            "import_media" => {
                if let Some(fingerprint) = result["media_fingerprint"].as_str() {
                    self.add_checked(&result["file_path"], false, Some(fingerprint));
                }
            }
            "get_media_assets" => {
                self.add(&result["peaks_path"], true);
                self.add(&result["playback_path"], true);
                if let Some(thumbnails) = result["thumbnails"].as_array() {
                    for thumbnail in thumbnails {
                        self.add(&thumbnail["path"], true);
                    }
                }
            }
            _ => {}
        }
    }

    fn add(&self, value: &Value, cache: bool) {
        self.add_checked(value, cache, None);
    }

    fn add_checked(&self, value: &Value, cache: bool, expected: Option<&str>) {
        let Some(path) = value
            .as_str()
            .map(Path::new)
            .filter(|path| path.is_absolute())
        else {
            return;
        };
        let Ok(canonical) = path.canonicalize() else {
            return;
        };
        let Ok(metadata) = canonical.metadata() else {
            return;
        };
        if !metadata.is_file() || !supported_file(&canonical, cache) {
            return;
        }
        if expected.is_some_and(|expected| !roughcut_core::Editor::media_metadata_matches(&canonical, &metadata, expected)) {
            return;
        }
        let Ok(modified) = metadata.modified() else { return; };
        if cache {
            let root = roughcut_core::store::data_dir().join("cache");
            if !root
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir())
                || !root
                    .canonicalize()
                    .is_ok_and(|root| canonical.starts_with(root))
            {
                return;
            }
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        self.0.lock().unwrap().insert(
            path.to_owned(),
            ApprovedMedia {
                canonical,
                bytes: metadata.len(),
                modified,
                #[cfg(unix)]
                identity: (metadata.dev(), metadata.ino()),
            },
        );
    }

    pub fn open(&self, requested: &Path) -> Option<(PathBuf, File)> {
        let approved = self.0.lock().unwrap().get(requested)?.clone();
        let canonical = requested.canonicalize().ok()?;
        if approved.canonical != canonical || !approved.matches(&canonical.metadata().ok()?) {
            return None;
        }
        let file = File::open(&canonical).ok()?;
        approved
            .matches(&file.metadata().ok()?)
            .then_some((canonical, file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approved_media_must_keep_its_original_contents_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("source.mov");
        std::fs::write(&media, b"source").unwrap();
        let access = MediaAccess::default();
        access.record("open_project", &json!({ "media": { "file_path": media } }));
        assert!(access.open(&media).is_some());

        std::fs::write(&media, b"changed source").unwrap();
        assert!(access.open(&media).is_none());
    }

    #[test]
    fn new_project_grants_only_its_media() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("source.mov");
        let other = dir.path().join("other.mov");
        std::fs::write(&media, []).unwrap();
        std::fs::write(&other, []).unwrap();

        let access = MediaAccess::default();
        access.record("import_media", &json!({ "file_path": media }));
        assert!(access.open(&media).is_none());
        access.record(
            "create_project",
            &json!({ "media": { "file_path": media } }),
        );
        assert_eq!(
            access.open(&media).map(|(path, _)| path),
            Some(media.canonicalize().unwrap())
        );
        assert!(access.open(&other).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn retargeted_alias_is_denied() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("source.mov");
        let other = dir.path().join("other.mov");
        let alias = dir.path().join("alias.mov");
        std::fs::write(&media, []).unwrap();
        std::fs::write(&other, []).unwrap();
        std::os::unix::fs::symlink(&media, &alias).unwrap();

        let access = MediaAccess::default();
        access.record("open_project", &json!({ "media": { "file_path": alias } }));
        assert_eq!(
            access.open(&alias).map(|(path, _)| path),
            Some(media.canonicalize().unwrap())
        );
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&other, &alias).unwrap();
        assert!(access.open(&alias).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn replacement_at_same_path_requires_new_approval() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mov");
        let replacement = dir.path().join("replacement.mov");
        std::fs::write(&source, b"original").unwrap();
        std::fs::write(&replacement, b"replacement").unwrap();

        let access = MediaAccess::default();
        let opened = json!({ "media": { "file_path": source } });
        access.record("open_project", &opened);
        let (_, original_handle) = access.open(&source).unwrap();

        std::fs::rename(&replacement, &source).unwrap();
        assert!(access.open(&source).is_none());
        assert_eq!(original_handle.metadata().unwrap().len(), 8);

        access.record("open_project", &opened);
        let (_, new_handle) = access.open(&source).unwrap();
        assert_eq!(new_handle.metadata().unwrap().len(), 11);
    }
}

fn supported_file(path: &Path, cache: bool) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "m4a" | "mp3" | "wav" | "aac" | "flac"
        | "ogg" | "opus" => true,
        "jpg" | "jpeg" => cache,
        "bin" => cache && path.file_name().is_some_and(|name| name == "peaks.bin"),
        _ => false,
    }
}
