//! Plain-text transcript of the CURRENT CUT: same inclusion rule as SRT
//! (excluded segments dropped), no timecodes. A new paragraph starts wherever
//! the cut skips something — a removed segment or a silence.

use crate::model::{Project, Transcript};

pub fn write(project: &Project, transcript: &Transcript) -> String {
    let tl = &project.timeline;
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for seg in &transcript.segments {
        let Some((_, _, text)) = super::srt::edited_segment(tl, seg) else {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
            continue;
        };
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(text.trim());
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    let mut out = paragraphs.join("\n\n");
    out.push('\n');
    out
}
