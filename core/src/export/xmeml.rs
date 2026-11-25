//! FCP7 "xmeml" XML — the interchange dialect Premiere Pro and DaVinci
//! Resolve import ("Final Cut Pro XML").

use super::{file_url, xml_escape};
use crate::model::{Media, Project};
use crate::time::{retained_frame_spans, source_frame_range, to_frame};
use super::timecode::parse_timecode;

const TIMELINE_START_TIMECODE: &str = "00:00:00:00";

#[derive(serde::Serialize)]
struct FrameSpan {
    source_in_frame: i64,
    source_out_frame: i64,
    timeline_in_frame: i64,
    timeline_out_frame: i64,
}

impl FrameSpan {
    fn clipped_to_source_end(&self, source_end: i64) -> Option<Self> {
        let source_out_frame = self.source_out_frame.min(source_end);
        (source_out_frame > self.source_in_frame).then(|| Self {
            source_in_frame: self.source_in_frame,
            source_out_frame,
            timeline_in_frame: self.timeline_in_frame,
            timeline_out_frame: self.timeline_in_frame + (source_out_frame - self.source_in_frame),
        })
    }
}

fn frame_spans(project: &Project, media: &Media) -> Vec<FrameSpan> {
    let ranges = project.timeline.included_clips().map(|clip| (clip.source_in, clip.source_out));
    retained_frame_spans(ranges, media.duration, media.frame_rate)
        .map(|(source_in, source_out, timeline_in, timeline_out)| FrameSpan {
            source_in_frame: source_in,
            source_out_frame: source_out,
            timeline_in_frame: timeline_in,
            timeline_out_frame: timeline_out,
        }).collect()
}

fn rate(fps: f64) -> (i64, bool) {
    (fps.round() as i64, (fps - fps.round()).abs() > 0.001)
}

pub(super) struct Manifest {
    pub text: String,
    pub cut_plan_version: String,
    pub timeline_frames: i64,
}

pub(super) fn manifest(project: &Project, media: &Media, source_fingerprint: Option<&str>, screen_fingerprint: Option<&str>, selected_stream: Option<usize>, xml_output: Option<(&str, &str, uuid::Uuid)>) -> serde_json::Result<Manifest> {
    use sha2::{Digest, Sha256};
    let (nominal, fractional) = rate(media.frame_rate);
    let (numerator, denominator) = if fractional { (nominal.saturating_mul(1000), 1001) } else { (nominal, 1) };
    let spans = frame_spans(project, media);
    let screen_frames = project.screen_media.as_ref().map(|screen| to_frame(screen.duration, media.frame_rate));
    let cut_plan = serde_json::to_vec(&(numerator, denominator, selected_stream, screen_frames, &spans))?;
    let cut_plan_version = format!("sha256:{:x}", Sha256::digest(cut_plan));
    let timeline_end = spans.last().map_or(0, |span| span.timeline_out_frame);
    let selected_audio = selected_stream.and_then(|index| media.audio_streams.iter().find(|audio| audio.index == index));
    let (channels, sample_rate) = match media.source_audio_tracks {
        Some(0) => (None, None),
        Some(tracks) if tracks > 1 => (
            selected_audio.and_then(|audio| audio.channels),
            selected_audio.and_then(|audio| audio.sample_rate),
        ),
        _ => (media.source_audio_channels, Some(media.audio_sample_rate)),
    };
    let xml_blocker = super::validate_resolve_timing(project, media, true).err().map(|error| error.to_string());
    let import_status = if xml_output.is_some() {
        "manual_import_pending"
    } else if xml_blocker.is_some() {
        "xml_unavailable"
    } else {
        "plan_only"
    };
    let text = serde_json::to_string_pretty(&serde_json::json!({
        "bridge_version": 1,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "roughcut_project_id": project.id,
        "resolve_project_id": null,
        "resolve_media_item_id": null,
        "cut_plan_version": cut_plan_version,
        "source_media_id": media.id,
        "source_media_fingerprint": source_fingerprint,
        "source_file_path": media.file_path,
        "screen_source": project.screen_media.as_ref().map(|screen| serde_json::json!({
            "media_id": screen.id,
            "file_path": screen.file_path,
            "fingerprint": screen_fingerprint,
            "timecode_start": parsed_source_timecode(screen).map(|(value, _, _)| value),
            "kept_spans": spans.iter()
                .filter_map(|span| span.clipped_to_source_end(to_frame(screen.duration, media.frame_rate)))
                .collect::<Vec<_>>(),
        })),
        "source_timecode_start": parsed_source_timecode(media).map(|(value, _, _)| value),
        "source_audio": {
            "tracks": media.source_audio_tracks,
            "transcript_stream_index": selected_stream,
            "channels": channels,
            "sample_rate": sample_rate,
        },
        "frame_rate": { "numerator": numerator, "denominator": denominator },
        "timeline_name": super::xml_text(&project.name),
        "timeline_timecode_start": TIMELINE_START_TIMECODE,
        "timeline_frames": timeline_end,
        "xml_output_path": xml_output.map(|(path, _, _)| path),
        "xml_sha256": xml_output.map(|(_, checksum, _)| checksum),
        "xml_export_id": xml_output.map(|(_, _, id)| id),
        "kept_spans": spans,
        "import_status": import_status,
        "xml_blocker": xml_blocker,
    }))?;
    Ok(Manifest { text, cut_plan_version, timeline_frames: timeline_end })
}

pub(super) fn source_frames(start: f64, end: f64, media: &Media) -> (i64, i64) {
    source_frame_range(start, end, media.duration, media.frame_rate)
}

fn parsed_source_timecode(media: &Media) -> Option<(&str, i64, bool)> {
    let value = media.source_timecode.as_deref()?;
    let (nominal, fractional) = rate(media.frame_rate);
    let (frame, drop_frame) = parse_timecode(value, nominal, fractional)?;
    Some((value, frame, drop_frame))
}

fn source_timecode(media: &Media) -> String {
    let Some((value, frame, drop_frame)) = parsed_source_timecode(media) else { return String::new() };
    let (nominal, fractional) = rate(media.frame_rate);
    let display = if drop_frame { "DF" } else { "NDF" };
    let ntsc = if fractional { "TRUE" } else { "FALSE" };
    format!("<timecode><rate><timebase>{nominal}</timebase><ntsc>{ntsc}</ntsc></rate><string>{value}</string><frame>{frame}</frame><displayformat>{display}</displayformat></timecode>")
}

pub fn write(project: &Project, media: &Media) -> String {
    let fps = media.frame_rate;
    let (timebase, fractional) = rate(fps);
    let ntsc = if fractional { "TRUE" } else { "FALSE" };
    let name = xml_escape(&project.name);
    let file_name = std::path::Path::new(&media.file_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| media.file_path.clone());
    let media_url = file_url(&media.file_path);
    let media_frames = to_frame(media.duration, fps).max(0);
    let channels = if media.source_audio_tracks == Some(0) {
        0
    } else {
        media.source_audio_channels.unwrap_or(2)
    };
    let audio_media = if channels > 0 {
        format!("<audio><samplecharacteristics><samplerate>{}</samplerate></samplecharacteristics><channelcount>{channels}</channelcount></audio>", media.audio_sample_rate)
    } else {
        String::new()
    };

    let mut video_items = String::new();
    let mut screen_items = String::new();
    let mut audio_items = String::new();
    let spans = frame_spans(project, media);
    let total_frames = spans.last().map_or(0, |span| span.timeline_out_frame);
    let screen = project.screen_media.as_ref();
    let screen_track_present = screen.is_some_and(|screen| {
        let screen_frames = to_frame(screen.duration, fps);
        spans.iter().any(|span| span.clipped_to_source_end(screen_frames).is_some())
    });
    let camera_track = if screen_track_present { 2 } else { 1 };
    let mut screen_file_written = false;
    for (index, span) in spans.iter().enumerate() {
        let item_index = index + 1;
        let in_f = span.source_in_frame;
        let out_f = span.source_out_frame;
        let record = span.timeline_in_frame;
        let file_def = if item_index == 1 {
            format!(
                "<file id=\"file-1\"><name>{fname}</name><pathurl>{url}</pathurl>\
                 <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
                 <duration>{media_dur}</duration>{timecode}\
                 <media><video><samplecharacteristics>\
                 <width>{w}</width><height>{h}</height>\
                 </samplecharacteristics></video>{audio_media}</media></file>",
                fname = xml_escape(&file_name),
                url = xml_escape(&media_url),
                media_dur = media_frames,
                timecode = source_timecode(media),
                w = media.width,
                h = media.height,
            )
        } else {
            "<file id=\"file-1\"/>".to_string()
        };
        let links = if channels == 0 { String::new() } else { format!(
            "<link><linkclipref>clipitem-v{item_index}</linkclipref><mediatype>video</mediatype>\
             <trackindex>{camera_track}</trackindex><clipindex>{item_index}</clipindex></link>\
             <link><linkclipref>clipitem-a{item_index}</linkclipref><mediatype>audio</mediatype>\
             <trackindex>1</trackindex><clipindex>{item_index}</clipindex><groupindex>1</groupindex></link>"
        ) };
        video_items.push_str(&format!(
            "<clipitem id=\"clipitem-v{n}\"><name>{fname}</name>\
             <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
             <start>{start}</start><end>{end}</end><in>{in_f}</in><out>{out_f}</out>\
             {file_def}{links}</clipitem>",
            n = item_index,
            fname = xml_escape(&file_name),
            start = record,
            end = span.timeline_out_frame,
        ));
        if channels > 0 {
            audio_items.push_str(&format!(
            "<clipitem id=\"clipitem-a{n}\"><name>{fname}</name>\
             <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
             <start>{start}</start><end>{end}</end><in>{in_f}</in><out>{out_f}</out>\
             <file id=\"file-1\"/>\
             <sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack>\
             {links}</clipitem>",
            n = item_index,
            fname = xml_escape(&file_name),
            start = record,
            end = span.timeline_out_frame,
            ));
        }
        // Dual-capture: the screen rides as a second video track with the
        // SAME record/source timings (shared clock — cuts apply to both).
        // No baked transforms: the finishing tool positions the face.
        if let Some(scr) = screen {
            let s_name = std::path::Path::new(&scr.file_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| scr.file_path.clone());
            let s_url = file_url(&scr.file_path);
            let s_file = if !screen_file_written {
                format!(
                    "<file id=\"file-2\"><name>{fname}</name><pathurl>{url}</pathurl>\
                     <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
                     <duration>{sdur}</duration>{timecode}\
                     <media><video><samplecharacteristics>\
                     <width>{w}</width><height>{h}</height>\
                     </samplecharacteristics></video></media></file>",
                    fname = xml_escape(&s_name),
                    url = xml_escape(&s_url),
                    sdur = to_frame(scr.duration, fps),
                    timecode = source_timecode(scr),
                    w = scr.width,
                    h = scr.height,
                )
            } else {
                "<file id=\"file-2\"/>".to_string()
            };
            // Clamp to the screen file's own length: importers reject
            // source ranges past a clip's media EOF.
            let s_total = to_frame(scr.duration, fps);
            if let Some(screen_span) = span.clipped_to_source_end(s_total) {
                screen_file_written = true;
                screen_items.push_str(&format!(
                    "<clipitem id=\"clipitem-s{n}\"><name>{fname}</name>\
                     <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
                     <start>{start}</start><end>{end}</end><in>{in_f}</in><out>{s_out}</out>\
                     {s_file}</clipitem>",
                    n = item_index,
                    fname = xml_escape(&s_name),
                    start = screen_span.timeline_in_frame,
                    end = screen_span.timeline_out_frame,
                    in_f = screen_span.source_in_frame,
                    s_out = screen_span.source_out_frame,
                ));
            }
        }
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xmeml>\n\
         <xmeml version=\"4\"><sequence id=\"sequence-1\"><name>{name}</name>\
         <duration>{total_frames}</duration>\
         <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
         <timecode><rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
         <string>{TIMELINE_START_TIMECODE}</string><frame>0</frame><displayformat>NDF</displayformat></timecode>\
         <media><video><format><samplecharacteristics>\
         <width>{w}</width><height>{h}</height>\
         <rate><timebase>{timebase}</timebase><ntsc>{ntsc}</ntsc></rate>\
         </samplecharacteristics></format>\
         {screen_track}<track>{video_items}</track></video>\
         {audio_track}</media>\
         </sequence></xmeml>\n",
        w = screen.map(|s| s.width).unwrap_or(media.width),
        h = screen.map(|s| s.height).unwrap_or(media.height),
        audio_track = if channels > 0 {
            format!("<audio><track>{audio_items}</track></audio>")
        } else {
            String::new()
        },
        screen_track = if screen_items.is_empty() {
            String::new()
        } else {
            // V1 (under) = screen; the camera track stacks above it.
            format!("<track>{screen_items}</track>")
        },
    )
}
