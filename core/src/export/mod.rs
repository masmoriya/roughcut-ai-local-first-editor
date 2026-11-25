//! Export / hand-off: NLE interchange (Premiere/Resolve via FCP7 xmeml,
//! Final Cut via fcpxml, CMX3600 EDL, OTIO JSON), SRT captions, plain-text
//! transcript, MP4 render.

pub mod edl;
pub mod fcpxml;
pub mod otio;
pub mod srt;
pub mod txt;
pub(crate) mod timecode;
pub mod xmeml;

use crate::engine::Editor;
use crate::error::{CoreError, Result};
use crate::events::{CoreEvent, EventSink, ProgressTask, SharedSink};
use crate::model::{ClipExportResult, PlaySpan, Project, Transcript};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

fn xml_text(s: &str) -> String {
    s.chars().map(|character| match character {
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}' => character,
        _ => '\u{FFFD}',
    }).collect()
}

pub fn xml_escape(s: &str) -> String {
    xml_text(s).replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\r', "&#13;")
}

/// A `file://` URL for a local path, percent-encoded so any file name
/// survives: spaces, `&`, `#`, `%`, quotes and non-ASCII letters included.
/// Only unreserved characters and `/` stay as they are.
pub fn file_url(path: &str) -> String {
    #[cfg(windows)]
    let windows_path = path.strip_prefix(r"\\?\").unwrap_or(path).replace('\\', "/");
    #[cfg(windows)]
    let path = windows_path.as_str();
    let mut url = String::from("file://");
    let drive_path = cfg!(windows) && path.as_bytes().get(0..3).is_some_and(|prefix| {
        prefix[0].is_ascii_alphabetic() && prefix[1] == b':' && prefix[2] == b'/'
    });
    if drive_path {
        url.push('/');
    }
    for (index, b) in path.bytes().enumerate() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') || (drive_path && index == 1) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

fn require_media(project: &Project) -> Result<&crate::model::Media> {
    project
        .media
        .as_ref()
        .ok_or_else(|| CoreError::InvalidArg("project has no media to export".into()))
}

/// Render a text-based target to a string (everything except mp4).
pub fn render_text_target(
    project: &Project,
    transcript: Option<&Transcript>,
    target: &str,
) -> Result<String> {
    let media = require_media(project)?;
    match target {
        // Premiere and Resolve both import FCP7 "xmeml" XML.
        "premiere_xml" => Ok(xmeml::write(project, media)),
        "resolve_xml" => {
            validate_resolve_timing(project, media, true)?;
            Ok(xmeml::write(project, media))
        }
        "resolve_manifest" => {
            validate_resolve_timing(project, media, false)?;
            let fingerprint = project.media_fingerprint.as_deref()
                .or_else(|| transcript.and_then(|value| value.source_fingerprint.as_deref()));
            Ok(xmeml::manifest(project, media, fingerprint, project.screen_media_fingerprint.as_deref(),
                transcript.and_then(|value| value.source_audio_stream_index), None)?.text)
        }
        "fcp_xml" => Ok(fcpxml::write(project, media)),
        "edl" => Ok(edl::write(project, media)),
        "otio" => otio::write(project, media),
        "srt" => {
            let t = transcript
                .ok_or_else(|| CoreError::InvalidArg("Import a timed transcript before exporting captions".into()))?;
            Ok(srt::write(project, t))
        }
        "txt" => {
            let t = transcript
                .ok_or_else(|| CoreError::InvalidArg("Import a timed transcript before exporting captions".into()))?;
            Ok(txt::write(project, t))
        }
        "transcript_json" => {
            let transcript = transcript
                .ok_or_else(|| CoreError::InvalidArg("Import a timed transcript before exporting transcript JSON.".into()))?;
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "source_frame_rate": media.frame_rate,
                "source_duration_seconds": media.duration,
                "source_timecode": media.source_timecode,
                "source_audio_channels": media.source_audio_channels,
                "source_audio_tracks": media.source_audio_tracks,
                "source_audio_sample_rate": media.audio_sample_rate,
                "transcript": transcript,
            }))?)
        }
        other => Err(CoreError::InvalidArg(format!("unknown export target {other}"))),
    }
}

fn validate_resolve_timing(project: &Project, media: &crate::model::Media, require_xml_audio: bool) -> Result<()> {
    if std::iter::once(media).chain(project.screen_media.iter()).any(|source| {
        source.average_frame_rate.is_some_and(|average| {
            let difference = (average - source.frame_rate).abs();
            !average.is_finite() || average <= 0.0
                || difference > 0.01 || difference * source.duration >= 0.5
        })
    }) {
        return Err(CoreError::InvalidArg(
            "The source reports inconsistent frame rates. Resolve XML requires constant-rate footage.".into(),
        ));
    }
    if let Some(tracks) = media.source_audio_tracks.filter(|tracks| require_xml_audio && *tracks > 1) {
        return Err(CoreError::InvalidArg(format!(
            "The source has {tracks} audio tracks. For Resolve XML, use a source with one mono or stereo audio track."
        )));
    }
    if require_xml_audio && media.source_audio_channels.is_some_and(|channels| channels > 2) {
        return Err(CoreError::InvalidArg("Resolve XML supports mono or stereo source audio in this editor".into()));
    }
    if require_xml_audio && media.source_audio_tracks == Some(1)
        && !matches!(media.source_audio_channels, Some(1 | 2))
    {
        return Err(CoreError::InvalidArg("Resolve XML requires a known mono or stereo channel count for the source audio".into()));
    }
    let fps = media.frame_rate;
    let nominal = fps.round();
    let integer_rate = (fps - nominal).abs() <= 0.001;
    let ntsc_rate = (fps - nominal * 1000.0 / 1001.0).abs() <= 0.001;
    if !fps.is_finite() || !(1.0..=f64::from(u32::MAX)).contains(&nominal) || (!integer_rate && !ntsc_rate) {
        return Err(CoreError::InvalidArg("The source frame rate cannot be represented in Resolve XML".into()));
    }
    let xml_rate = if integer_rate { nominal } else { nominal * 1000.0 / 1001.0 };
    if std::iter::once(media).chain(project.screen_media.iter()).any(|source| {
        !source.duration.is_finite() || source.duration <= 0.0
            || (source.frame_rate - xml_rate).abs() * source.duration >= 0.5
    }) {
        return Err(CoreError::InvalidArg("The source rate would drift from the Resolve XML rate. Relink a constant-frame-rate source before exporting.".into()));
    }
    if !project.audio_offset_s.is_finite() || project.audio_offset_s.abs() > 0.001 {
        return Err(CoreError::InvalidArg("Resolve XML requires the source audio offset to be zero".into()));
    }
    if project.screen_media.as_ref().is_some_and(|screen| {
        !screen.frame_rate.is_finite() || (screen.frame_rate - fps).abs() > 0.001
    }) {
        return Err(CoreError::InvalidArg("Resolve XML requires the camera and screen recording to have the same frame rate".into()));
    }
    if !project.timeline.included_clips().any(|clip| {
        let (start, end) = xmeml::source_frames(clip.source_in, clip.source_out, media);
        end > start
    }) {
        return Err(CoreError::InvalidArg("Restore at least one frame before exporting the timeline".into()));
    }
    Ok(())
}

pub fn default_extension(target: &str) -> &'static str {
    match target {
        "premiere_xml" | "resolve_xml" => "xml",
        "fcp_xml" => "fcpxml",
        "transcript_json" | "resolve_manifest" => "json",
        "edl" => "edl",
        "otio" => "otio",
        "srt" => "srt",
        "txt" => "txt",
        "mp4" => "mp4",
        _ => "txt",
    }
}

/// Expand a leading `~/` (or a bare `~`) — callers (UI, MCP clients)
/// naturally write `~/Downloads/...`, and an unexpanded tilde becomes a
/// literal directory.
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

#[derive(serde::Serialize)]
pub(crate) struct ExportReceipt {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_timeline: Option<ResolveExportReceipt>,
}

#[derive(serde::Serialize)]
pub(crate) struct ResolveExportReceipt {
    name: String,
    source_path: Option<String>,
    source_fingerprint: Option<String>,
    timeline_frames: i64,
    xml_sha256: String,
    cut_plan_version: String,
    manifest: String,
}

impl Editor {
    /// The `export` tool: writes the target file and returns its path.
    pub async fn export(&self, project_id: Uuid, target: &str, out_path: &str) -> Result<String> {
        Ok(self.export_with_receipt(project_id, target, out_path).await?.path)
    }

    pub(crate) async fn export_with_receipt(&self, project_id: Uuid, target: &str, out_path: &str) -> Result<ExportReceipt> {
        check_target(target)?;
        // The project's timeline is the clip's while one is open.
        let (mut project, transcript) = self.snapshot(project_id)?;
        if project.resolve_timeline.is_some() && !matches!(target, "srt" | "txt" | "transcript_json") {
            return Err(CoreError::InvalidArg("Use the Resolve cut plan for a bound timeline".into()));
        }
        let mut source_fingerprint = None;
        let mut screen_fingerprint = None;
        if project.clip_session.is_some() {
            return Err(CoreError::InvalidArg("Save or cancel the clip edit before exporting the whole video".into()));
        }
        let path = PathBuf::from(expand_tilde(out_path));
        let parent = path.parent().filter(|parent| parent.is_absolute()).ok_or_else(|| {
            CoreError::InvalidArg("Choose an absolute export path".into())
        })?;
        validate_export_extension(&path, target)?;
        let parent = parent.canonicalize()?;
        if !parent.is_dir() {
            return Err(CoreError::InvalidArg("Choose an export directory".into()));
        }
        let destination = parent.join(path.file_name().unwrap());
        match destination.symlink_metadata() {
            Ok(_) => return Err(export_exists()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if matches!(target, "resolve_xml" | "resolve_manifest") {
            if !self.demo_mode() {
                require_media(&project)?;
                for (index, media) in project.media.iter_mut().chain(project.screen_media.iter_mut()).enumerate() {
                    let expected_fingerprint = if index == 0 {
                        project.media_fingerprint.as_deref()
                            .or_else(|| transcript.as_ref().and_then(|value| value.source_fingerprint.as_deref()))
                    } else {
                        project.screen_media_fingerprint.as_deref()
                    };
                    let (current, fingerprint) = self.probe_export_source(&media.file_path, expected_fingerprint).await?;
                    if current.average_frame_rate.is_none() {
                        return Err(CoreError::InvalidArg(
                            "Resolve export requires a source with a reported average frame rate".into(),
                        ));
                    }
                    if index == 0 {
                        if transcript.as_ref().and_then(|value| value.source_audio_stream_index)
                            .is_some_and(|index| !current.audio_streams.iter().any(|audio| audio.index == index))
                        {
                            return Err(CoreError::InvalidArg(
                                "The transcript dialogue track is missing from the source media".into(),
                            ));
                        }
                        source_fingerprint = Some(fingerprint);
                    } else {
                        screen_fingerprint = Some(fingerprint);
                    }
                    if (current.frame_rate - media.frame_rate).abs() > 0.001
                        || (current.duration - media.duration).abs() > 1.0 / current.frame_rate
                    {
                        return Err(CoreError::InvalidArg("The source timing changed. Relink the media before exporting.".into()));
                    }
                    media.file_path = current.file_path;
                    media.average_frame_rate = current.average_frame_rate;
                    media.width = current.width;
                    media.height = current.height;
                    media.audio_sample_rate = current.audio_sample_rate;
                    media.source_audio_tracks = current.source_audio_tracks;
                    media.source_audio_channels = current.source_audio_channels;
                    media.audio_streams = current.audio_streams;
                    media.source_timecode = current.source_timecode;
                }
            }
        }
        {
            if target == "mp4" {
                let staging = tempfile::tempdir_in(&parent)?;
                let file = staging.path().join("render.mp4");
                let temporary = file.to_str().ok_or_else(|| {
                    CoreError::InvalidArg("Choose an export directory with a Unicode path".into())
                })?;
                let pieces = vec![project.timeline.play_spans()];
                let sink: SharedSink = Arc::new(ProjectExportSink {
                    inner: self.sink(),
                    project_id,
                });
                self.write_export(&project, transcript.as_ref(), target, temporary, &pieces, None, &sink).await?;
                std::fs::File::open(&file)?.sync_all()?;
                publish_export(&file, &destination).map_err(export_write_error)?;
                return Ok(ExportReceipt { path: destination.to_string_lossy().into_owned(), resolve_timeline: None });
            }
            let xml_export_id = (target == "resolve_xml").then(Uuid::new_v4);
            if let Some(id) = xml_export_id {
                project.name = resolve_timeline_name(&project.name, id);
            }
            let content = if target == "resolve_manifest" && source_fingerprint.is_some() {
                let media = require_media(&project)?;
                validate_resolve_timing(&project, media, false)?;
                xmeml::manifest(&project, media, source_fingerprint.as_deref(), screen_fingerprint.as_deref(),
                    transcript.as_ref().and_then(|value| value.source_audio_stream_index), None)?.text
            } else {
                render_text_target(&project, transcript.as_ref(), target)?
            };
            let path = destination.to_string_lossy().into_owned();
            let resolve_timeline = if let Some(id) = xml_export_id {
                use sha2::{Digest, Sha256};
                let media = require_media(&project)?;
                let xml_sha256 = format!("{:x}", Sha256::digest(content.as_bytes()));
                let manifest = xmeml::manifest(&project, media, source_fingerprint.as_deref(), screen_fingerprint.as_deref(),
                    transcript.as_ref().and_then(|value| value.source_audio_stream_index),
                    Some((&path, &xml_sha256, id)))?;
                Some(ResolveExportReceipt {
                    name: xml_text(&project.name),
                    timeline_frames: manifest.timeline_frames,
                    source_path: Some(media.file_path.clone()),
                    manifest: manifest.text,
                    cut_plan_version: manifest.cut_plan_version,
                    xml_sha256,
                    source_fingerprint: source_fingerprint.clone(),
                })
            } else {
                None
            };
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            file.write_all(content.as_bytes())?;
            file.as_file().sync_all()?;
            for (media, fingerprint) in [
                (project.media.as_ref(), source_fingerprint.as_deref()),
                (project.screen_media.as_ref(), screen_fingerprint.as_deref()),
            ] {
                if let (Some(media), Some(fingerprint)) = (media, fingerprint) {
                    self.verify_export_source(&media.file_path, fingerprint)?;
                }
            }
            publish_export(file.path(), &destination).map_err(export_write_error)?;
            Ok(ExportReceipt { path, resolve_timeline })
        }
    }

    /// Export one saved clip: the same writers, run over the clip's
    /// timeline. The NLE sequence is named after the clip; a vertical clip's
    /// MP4 renders 9:16.
    pub async fn export_clip(
        &self,
        project_id: Uuid,
        clip_id: Uuid,
        target: &str,
        out_path: &str,
    ) -> Result<String> {
        check_target(target)?;
        let path = PathBuf::from(expand_tilde(out_path));
        validate_export_extension(&path, target)?;
        let parent = path.parent().filter(|parent| parent.is_absolute()).ok_or_else(|| {
            CoreError::InvalidArg("Choose an absolute export path".into())
        })?.canonicalize()?;
        let name = path.file_name().ok_or_else(|| CoreError::InvalidArg("Choose an export filename".into()))?;
        let destination = parent.join(name);
        if destination.symlink_metadata().is_ok() {
            return Err(export_exists());
        }
        let staging = tempfile::tempdir_in(&parent)?;
        let file = staging.path().join(format!("clip.{}", default_extension(target)));
        let temporary = file.to_str().ok_or_else(|| {
            CoreError::InvalidArg("Choose an export directory with a Unicode path".into())
        })?;
        self.export_clip_with(project_id, clip_id, target, temporary, &self.sink()).await?;
        std::fs::File::open(&file)?.sync_all()?;
        publish_export(&file, &destination).map_err(export_write_error)?;
        Ok(destination.to_string_lossy().into_owned())
    }

    async fn export_clip_with(
        &self,
        project_id: Uuid,
        clip_id: Uuid,
        target: &str,
        out_path: &str,
        sink: &SharedSink,
    ) -> Result<String> {
        if target == "transcript_json" {
            return Err(CoreError::InvalidArg("Export the timed transcript from the source project".into()));
        }
        let clip = self.get_clip(project_id, clip_id)?;
        let (mut project, transcript) = self.snapshot(project_id)?;
        let pieces = clip.piece_spans();
        project.name = format!("{} — {}", project.name, clip.title);
        if target == "resolve_xml" {
            project.name = resolve_timeline_name(&project.name, Uuid::new_v4());
        }
        let vertical = (clip.format == crate::model::ClipFormat::Vertical).then_some(clip.framing());
        project.timeline = clip.timeline;
        self.write_export(&project, transcript.as_ref(), target, out_path, &pieces, vertical, sink).await
    }

    /// Export several saved clips into one folder, one file each, named
    /// `NN-<title>.<ext>` in the order given. Never overwrites: a name that
    /// exists gets `-2`, `-3`, …. One clip failing doesn't stop the rest; each
    /// result carries its path or its error. Progress is one `export_clips`
    /// task over the whole batch.
    pub async fn export_clips(
        &self,
        project_id: Uuid,
        clip_ids: &[Uuid],
        out_dir: &str,
        target: &str,
    ) -> Result<Vec<ClipExportResult>> {
        if clip_ids.is_empty() {
            return Err(CoreError::InvalidArg("export_clips needs at least one clip id".into()));
        }
        check_target(target)?;
        // An unknown project is one error, not one per clip (and no folder).
        self.with_project(project_id, |_, _| Ok(()))?;
        let dir = PathBuf::from(expand_tilde(out_dir));
        std::fs::create_dir_all(&dir)?;
        let dir = dir.canonicalize()?;
        let staging = tempfile::tempdir_in(&dir)?;
        let n = clip_ids.len();
        let ext = default_extension(target);
        let mut results = Vec::with_capacity(n);
        for (i, &clip_id) in clip_ids.iter().enumerate() {
            let title = match self.get_clip(project_id, clip_id) {
                Ok(c) => c.title,
                Err(e) => {
                    results.push(ClipExportResult { clip_id, path: None, error: Some(e.to_string()) });
                    continue;
                }
            };
            let name = batch_file_name(i + 1, n, &title, ext);
            let temporary = staging.path().join(&name);
            let sink: SharedSink = Arc::new(BatchSink {
                inner: self.sink(),
                project_id,
                index: i,
                total: n,
                title: title.clone(),
            });
            batch_progress(&self.sink(), project_id, i as f64 / n as f64, format!("{}/{n} · {title}", i + 1));
            let out = async {
                let path = temporary.to_str().ok_or_else(|| {
                    CoreError::InvalidArg("Choose an export directory with a Unicode path".into())
                })?;
                self.export_clip_with(project_id, clip_id, target, path, &sink).await?;
                std::fs::File::open(&temporary)?.sync_all()?;
                publish_batch_file(&temporary, &dir, &name)
            }.await;
            results.push(match out {
                Ok(path) => ClipExportResult { clip_id, path: Some(path.to_string_lossy().into_owned()), error: None },
                Err(e) => ClipExportResult { clip_id, path: None, error: Some(e.to_string()) },
            });
            let _ = std::fs::remove_file(&temporary);
        }
        let ok = results.iter().filter(|r| r.path.is_some()).count();
        batch_progress(&self.sink(), project_id, 1.0, format!("exported {ok} of {n}"));
        Ok(results)
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_export(
        &self,
        project: &Project,
        transcript: Option<&Transcript>,
        target: &str,
        out_path: &str,
        pieces: &[Vec<PlaySpan>],
        vertical_crop: Option<crate::model::ClipFraming>,
        sink: &SharedSink,
    ) -> Result<String> {
        let out_path = &expand_tilde(out_path);
        let verify_sources = || -> Result<()> {
            let expected = project.media_fingerprint.as_deref()
                .or_else(|| transcript.and_then(|value| value.source_fingerprint.as_deref()));
            if let (Some(media), Some(expected)) = (&project.media, expected) {
                self.verify_export_source(&media.file_path, expected)?;
            }
            if let (Some(screen), Some(expected)) =
                (&project.screen_media, project.screen_media_fingerprint.as_deref())
            {
                self.verify_export_source(&screen.file_path, expected)?;
            }
            Ok(())
        };
        if matches!(target, "mp4" | "resolve_xml" | "resolve_manifest") {
            verify_sources()?;
        }
        if matches!(target, "resolve_xml" | "resolve_manifest") && !self.demo_mode() {
            let media = require_media(project)?;
            if std::iter::once(media).chain(project.screen_media.iter())
                .any(|source| source.average_frame_rate.is_none())
            {
                return Err(CoreError::InvalidArg(
                    "Resolve export requires a reported average frame rate. Relink the source to refresh its metadata.".into(),
                ));
            }
        }
        if target == "mp4" {
            let media = require_media(project)?.clone();
            let job = crate::adapters::video::RenderJob {
                media: &media,
                pieces,
                out_path,
                audio_offset_s: project.audio_offset_s,
                audio_stream_index: transcript.and_then(|value| value.source_audio_stream_index),
                screen: project.screen_media.as_ref().map(|m| (m, &project.layout)),
                vertical_crop,
            };
            self.video().render_mp4(&job, sink).await?;
            verify_sources()?;
            return Ok(out_path.to_string());
        }
        let content = render_text_target(project, transcript, target)?;
        if let Some(parent) = std::path::Path::new(out_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(out_path, content)?;
        Ok(out_path.to_string())
    }

}

fn export_exists() -> CoreError {
    CoreError::InvalidArg("A file already exists at that path. Choose a new filename.".into())
}

fn export_write_error(error: std::io::Error) -> CoreError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        export_exists()
    } else {
        error.into()
    }
}

/// Fail early on a target no writer knows (before creating folders).
fn resolve_timeline_name(name: &str, export_id: Uuid) -> String {
    format!("{name} - {}", &export_id.simple().to_string()[..12])
}

fn validate_export_extension(path: &std::path::Path, target: &str) -> Result<()> {
    let extension = default_extension(target);
    if path.extension().is_some_and(|actual| {
        actual.eq_ignore_ascii_case(extension) || (target == "fcp_xml" && actual.eq_ignore_ascii_case("xml"))
    }) {
        return Ok(());
    }
    Err(CoreError::InvalidArg(format!("Choose a .{extension} filename")))
}

pub fn check_target(target: &str) -> Result<()> {
    match target {
        "premiere_xml" | "resolve_xml" | "resolve_manifest" | "fcp_xml" | "edl" | "otio" | "srt" | "txt" | "mp4" | "transcript_json" => Ok(()),
        other => Err(CoreError::InvalidArg(format!("unknown export target {other}"))),
    }
}

/// `NN-<slug>.<ext>`: `NN` is the 1-based position, two digits (three past
/// 99); the slug is the title in lowercase ASCII letters and digits, every
/// other run of characters one `-`, at most 40 characters ("clip" if empty).
fn batch_file_name(position: usize, total: usize, title: &str, ext: &str) -> String {
    let width = if total > 99 { 3 } else { 2 };
    format!("{position:0width$}-{}.{ext}", slug(title))
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out.trim_matches('-').chars().take(40).collect();
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "clip".into()
    } else {
        out
    }
}

/// Publish a completed clip under the first free name without replacing it.
fn publish_batch_file(temporary: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    for k in 1.. {
        let path = if k == 1 { dir.join(name) } else { dir.join(format!("{stem}-{k}{ext}")) };
        match publish_export(temporary, &path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("an unused name exists")
}

fn publish_export(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(temporary, destination) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => {}
    }
    #[cfg(target_os = "macos")]
    match rename_exclusive(temporary, destination) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Err(error),
        Err(_) => {}
    }
    // ExFAT and some mounted shares cannot create hard links. Copy the
    // completed file through an exclusive handle so existing files stay safe.
    let mut source = std::fs::File::open(temporary)?;
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(destination)?;
    let result = std::io::copy(&mut source, &mut output).and_then(|_| output.sync_all());
    drop(output);
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(target_os = "macos")]
fn rename_exclusive(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    const RENAME_EXCL: c_uint = 0x00000004;
    unsafe extern "C" {
        fn renamex_np(source: *const c_char, destination: *const c_char, flags: c_uint) -> c_int;
    }

    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // Both pointers are valid NUL-terminated strings for the whole call.
    let status = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    if status == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn batch_progress(sink: &SharedSink, project_id: Uuid, fraction: f64, message: String) {
    crate::events::send(sink, CoreEvent::progress(ProgressTask::ExportClips, Some(project_id), fraction, message));
}

struct ProjectExportSink {
    inner: SharedSink,
    project_id: Uuid,
}

impl EventSink for ProjectExportSink {
    fn emit(&self, event: &str, mut payload: serde_json::Value) {
        if event == "progress" && payload["task"] == "export" {
            payload["project_id"] = serde_json::Value::String(self.project_id.to_string());
        }
        self.inner.emit(event, payload);
    }
}

/// Folds one clip's render progress into the batch's: an `export` event at
/// fraction `f` for clip `i` of `n` becomes an `export_clips` event at
/// `(i + f) / n`, its message prefixed with the position and title. The
/// per-clip "render complete" is dropped (the batch sends its own end), and
/// every other event passes through.
struct BatchSink {
    inner: SharedSink,
    project_id: Uuid,
    index: usize,
    total: usize,
    title: String,
}

impl EventSink for BatchSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let is_render = event == "progress" && payload["task"] == "export";
        if !is_render {
            return self.inner.emit(event, payload);
        }
        let fraction = payload["fraction"].as_f64().unwrap_or(0.0);
        let message = payload["message"].as_str().unwrap_or("");
        if fraction >= 1.0 {
            return;
        }
        let n = self.total as f64;
        batch_progress(
            &self.inner,
            self.project_id,
            ((self.index as f64 + fraction) / n).clamp(0.0, 1.0),
            format!("{}/{} · {} · {message}", self.index + 1, self.total, self.title),
        );
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn xml_text_replaces_forbidden_characters_and_escapes_markup() {
        assert_eq!(
            xml_escape("A\0\u{1F} & < > \" ' café\n"),
            "A�� &amp; &lt; &gt; &quot; &apos; café\n",
        );
        assert_eq!(xml_escape("\u{FFFE}\u{FFFF}🎬\t\r"), "��🎬\t&#13;");
    }

    #[test]
    #[cfg(unix)]
    fn export_preserves_existing_destination_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let staging = directory.path().join("staged.xml");
        let existing = directory.path().join("existing.xml");
        let destination = directory.path().join("result.xml");
        std::fs::write(&staging, b"<export/>").unwrap();
        std::fs::write(&existing, b"<existing/>").unwrap();
        std::os::unix::fs::symlink(&existing, &destination).unwrap();
        let error = publish_export(&staging, &destination).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&existing).unwrap(), b"<existing/>");
        assert_eq!(std::fs::read_link(&destination).unwrap(), existing);
    }

    #[test]
    fn concurrent_exports_preserve_the_first_published_file() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.xml");
        let second = directory.path().join("second.xml");
        let output = directory.path().join("output.xml");
        std::fs::write(&first, b"<first/>").unwrap();
        std::fs::write(&second, b"<second/>").unwrap();
        let (first_result, second_result) = std::thread::scope(|scope| {
            let first = scope.spawn(|| publish_export(&first, &output));
            let second = scope.spawn(|| publish_export(&second, &output));
            (first.join().unwrap(), second.join().unwrap())
        });
        let (loser, expected): (_, &[u8]) = if first_result.is_ok() {
            (second_result, b"<first/>")
        } else {
            second_result.unwrap();
            (first_result, b"<second/>")
        };
        assert_eq!(loser.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }

    #[test]
    fn file_urls_encode_every_character_a_path_can_hold() {
        assert_eq!(file_url("/Users/me/clip.mp4"), "file:///Users/me/clip.mp4");
        assert_eq!(
            file_url("/Users/me/Tom & Jerry #2 100%.mov"),
            "file:///Users/me/Tom%20%26%20Jerry%20%232%20100%25.mov"
        );
        assert_eq!(file_url("/v/\"a\"'b'<c>.mp4"), "file:///v/%22a%22%27b%27%3Cc%3E.mp4");
        assert_eq!(file_url("/v/café.mp4"), "file:///v/caf%C3%A9.mp4");
        #[cfg(windows)]
        assert_eq!(file_url(r"\\?\C:\Clips\A, B.mov"), "file:///C:/Clips/A%2C%20B.mov");
    }

    #[test]
    fn batch_names_pad_slug_and_never_collide() {
        assert_eq!(batch_file_name(3, 6, "Why Timelines Are a Trap!", "mp4"), "03-why-timelines-are-a-trap.mp4");
        assert_eq!(batch_file_name(7, 120, "x", "srt"), "007-x.srt");
        assert_eq!(batch_file_name(1, 1, "¿Qué? — ", "mp4"), "01-qu.mp4");
        assert_eq!(batch_file_name(1, 1, "日本語", "mp4"), "01-clip.mp4");
        let long = "a".repeat(39) + " bcd";
        assert_eq!(slug(&long), "a".repeat(39), "cut to 40, no trailing dash");

        let dir = std::env::temp_dir().join(format!("roughcut-batch-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("staged.mp4");
        std::fs::write(&source, b"complete clip").unwrap();
        assert_eq!(publish_batch_file(&source, &dir, "01-a.mp4").unwrap(), dir.join("01-a.mp4"));
        assert_eq!(std::fs::read(dir.join("01-a.mp4")).unwrap(), b"complete clip");
        assert_eq!(publish_batch_file(&source, &dir, "01-a.mp4").unwrap(), dir.join("01-a-2.mp4"));
        assert_eq!(publish_batch_file(&source, &dir, "01-a.mp4").unwrap(), dir.join("01-a-3.mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct Capture(Mutex<Vec<(String, serde_json::Value)>>);
    impl EventSink for Capture {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    #[test]
    fn batch_sink_folds_render_progress_into_the_batch() {
        let capture = Arc::new(Capture(Mutex::new(vec![])));
        let pid = Uuid::new_v4();
        let sink: SharedSink =
            Arc::new(BatchSink { inner: capture.clone(), project_id: pid, index: 1, total: 4, title: "Hook".into() });
        crate::events::send(&sink, CoreEvent::progress(ProgressTask::Export, None, 0.5, "rendering MP4… 50%"));
        crate::events::send(&sink, CoreEvent::progress(ProgressTask::Export, None, 1.0, "render complete"));
        crate::events::send(&sink, CoreEvent::ClipsChanged { project_id: pid });
        let got = capture.0.lock().unwrap();
        assert_eq!(got.len(), 2, "render complete is dropped; other events pass");
        assert_eq!(got[0].1["task"], "export_clips");
        assert!((got[0].1["fraction"].as_f64().unwrap() - 0.375).abs() < 1e-9);
        assert_eq!(got[0].1["message"], "2/4 · Hook · rendering MP4… 50%");
        assert_eq!(got[0].1["project_id"], pid.to_string());
        assert_eq!(got[1].0, "clips-changed");
    }

}
