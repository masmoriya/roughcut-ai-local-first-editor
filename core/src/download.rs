//! One streaming download: .part file → optional sha256 verify → atomic
//! rename, with throttled per-percent progress. Three features had divergent
//! copies (whisper models had hashing but the others didn't; the llama
//! tarball buffered entirely in RAM with no progress at all).

use crate::error::{CoreError, Result};
use crate::events::{send, CoreEvent, ProgressTask, SharedSink};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;

pub(crate) fn verify_file(path: &Path, expected: &str) -> Result<()> {
    if !path.symlink_metadata()?.is_file() {
        return Err(CoreError::Unavailable("Model must be a regular file".into()));
    }
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 { break; }
        digest.update(&buffer[..read]);
    }
    if format!("{:x}", digest.finalize()) != expected {
        return Err(CoreError::Unavailable("Model checksum failed. Verify or repair the model in Setup.".into()));
    }
    Ok(())
}

pub struct DownloadOpts<'a> {
    pub task: ProgressTask,
    /// Pinned sha256 (hex); verified before the rename when present.
    pub sha256: Option<&'a str>,
    /// Fallback size for progress when the server omits content-length.
    pub approx_bytes: Option<u64>,
}

/// Stream `url` to `dest`. Never leaves a half-written file at `dest`:
/// writes to a private temporary file, verifies, then renames.
pub async fn download_to(
    sink: &SharedSink,
    url: &str,
    dest: &Path,
    opts: DownloadOpts<'_>,
) -> Result<()> {
    let parent = dest.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .read_timeout(std::time::Duration::from_secs(60))
        .user_agent("roughcut")
        .build()
        .map_err(|e| CoreError::Unavailable(format!("download client: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CoreError::Unavailable(format!("download: {e}")))?;
    if !resp.status().is_success() {
        return Err(CoreError::Unavailable(format!("download: HTTP {}", resp.status())));
    }
    let total = resp.content_length().or(opts.approx_bytes).unwrap_or(0);

    let max_bytes = opts.approx_bytes.map(|bytes| bytes.saturating_mul(2)).unwrap_or(4 * 1024 * 1024 * 1024);
    if resp.content_length().is_some_and(|size| size > max_bytes) {
        return Err(CoreError::Unavailable("Model download exceeds its size limit".into()));
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    let mut last_percent = 0u64;
    let mut resp = resp;
    while let Some(chunk) =
        resp.chunk().await.map_err(|e| CoreError::Unavailable(format!("download: {e}")))?
    {
        written = written.saturating_add(chunk.len() as u64);
        if written > max_bytes {
            return Err(CoreError::Unavailable("Model download exceeds its size limit".into()));
        }
        file.write_all(&chunk)?;
        if opts.sha256.is_some() {
            hasher.update(&chunk);
        }
        if total > 0 {
            let percent = written * 100 / total;
            if percent > last_percent {
                last_percent = percent;
                send(
                    sink,
                    CoreEvent::progress(
                        opts.task,
                        None,
                        (written as f64 / total as f64).min(0.99),
                        format!("{} MB / {} MB", written / 1_048_576, total / 1_048_576),
                    ),
                );
            }
        }
    }
    if let Some(expected) = opts.sha256 {
        let digest = format!("{:x}", hasher.finalize());
        if digest != expected {
            return Err(CoreError::Other(format!(
                "download corrupted (sha256 {digest} != expected {expected}); removed — try again"
            )));
        }
    }
    file.as_file().sync_all()?;
    file.persist(dest).map_err(|error| error.error)?;
    Ok(())
}
