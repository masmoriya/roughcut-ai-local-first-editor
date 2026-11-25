//! SRT captions for the CURRENT CUT: excluded segments are dropped and times
//! are remapped to output time.

use crate::model::{Project, Timeline, Transcript, TranscriptSegment};
use std::borrow::Cow;

fn srt_time(seconds: f64) -> String {
    let ms = (seconds * 1000.0).round() as i64;
    let (h, m, s, milli) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02},{milli:03}")
}

pub(super) fn edited_segment<'a>(timeline: &Timeline, segment: &'a TranscriptSegment) -> Option<(f64, f64, Cow<'a, str>)> {
    if segment.is_silence || segment.text.trim().is_empty() { return None; }
    let retained = |start: f64, end: f64| {
        let midpoint = (start + end) / 2.0;
        timeline.clips.iter().any(|clip| clip.included && clip.source_in <= midpoint && midpoint < clip.source_out)
    };
    if segment.words.is_empty() {
        return retained(segment.start, segment.end).then_some((segment.start, segment.end, Cow::Borrowed(&segment.text)));
    }
    let words: Vec<_> = segment.words.iter().filter(|word| retained(word.start, word.end)).collect();
    let first = words.first()?;
    let last = words.last()?;
    if words.len() == segment.words.len() {
        return Some((segment.start, segment.end, Cow::Borrowed(&segment.text)));
    }
    let text = words.iter().map(|word| word.text.trim()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some((first.start, last.end, Cow::Owned(text)))
}

pub fn write(project: &Project, transcript: &Transcript) -> String {
    let tl = &project.timeline;
    let mut out = String::new();
    let mut n = 1;
    for seg in &transcript.segments {
        let Some((start, end, text)) = edited_segment(tl, seg) else { continue; };
        let start = tl.source_to_output(start.max(0.0));
        let end = tl.source_to_output(end);
        if end <= start { continue; }
        let (start, end) = (srt_time(start), srt_time(end));
        if start == end {
            continue;
        }
        let text = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
        out.push_str(&format!("{n}\n{start} --> {end}\n{text}\n\n"));
        n += 1;
    }
    out
}
