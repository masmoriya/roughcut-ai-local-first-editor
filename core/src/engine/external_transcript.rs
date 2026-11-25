use crate::error::{CoreError, Result};
use crate::model::{Transcript, TranscriptSegment, Word};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct ExternalTranscript {
    segments: Vec<ExternalSegment>,
}

#[derive(Deserialize)]
struct ExternalSegment {
    start: f64,
    end: f64,
    text: String,
    speaker: Option<String>,
    #[serde(default)]
    words: Vec<ExternalWord>,
}

#[derive(Deserialize)]
struct ExternalWord {
    #[serde(alias = "word")]
    text: String,
    start: f64,
    end: f64,
    confidence: Option<f64>,
}

fn srt_time(value: &str) -> Option<f64> {
    let mut parts = value.trim().split(':');
    let hours: u32 = parts.next()?.parse().ok()?;
    let minutes: u32 = parts.next()?.parse().ok()?;
    let (seconds, millis) = parts.next()?.split_once(',')?;
    let second: u32 = seconds.parse().ok()?;
    if minutes >= 60 || second >= 60 || millis.len() != 3 || parts.next().is_some() {
        return None;
    }
    let millis: u32 = millis.parse().ok()?;
    if millis >= 1000 { return None; }
    Some(f64::from(hours) * 3600.0 + f64::from(minutes * 60 + second) + f64::from(millis) / 1000.0)
}

fn parse_srt_cues(text: &str) -> std::result::Result<Vec<(f64, f64, String)>, usize> {
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut segments: Vec<(f64, f64, String)> = Vec::new();
    let lines: Vec<_> = normalized.lines().collect();
    for cue in lines.split(|line| line.trim().is_empty()).filter(|cue| !cue.is_empty()) {
        let cue_number = segments.len() + 1;
        let timing_index = usize::from(cue[0].trim().bytes().all(|byte| byte.is_ascii_digit()));
        let (start, end) = cue.get(timing_index).and_then(|line| line.split_once("-->"))
            .and_then(|(start, end)| {
                let mut fields = end.split_whitespace();
                let end = srt_time(fields.next()?)?;
                let valid_position = fields.all(|field| {
                    field.split_once(':').is_some_and(|(key, value)| {
                        matches!(key, "X1" | "X2" | "Y1" | "Y2") && value.parse::<u32>().is_ok()
                    })
                });
                valid_position.then_some((srt_time(start)?, end))
            })
            .ok_or(cue_number)?;
        let text = cue[timing_index + 1..].join("\n");
        if end <= start || text.trim().is_empty() || segments.last().is_some_and(|previous| start < previous.0) {
            return Err(cue_number);
        }
        segments.push((start, end, text));
    }
    Ok(segments)
}

#[cfg(test)]
mod srt_tests {
    use super::parse_srt_cues;

    #[test]
    fn srt_accepts_bom_crlf_and_multiline_text() {
        let cues = parse_srt_cues("\u{feff}1\r\n00:00:01,250 --> 00:00:02,500\r\nFirst line\r\nSecond line\r\n").unwrap();
        assert_eq!(cues, vec![(1.25, 2.5, "First line\nSecond line".into())]);
    }

    #[test]
    fn srt_reports_first_invalid_cue() {
        let input = "1\n00:00:01,000 --> 00:00:02,000\nFirst cue\n\n2\n00:00:03,000 --> 00:00:02,000\nReversed cue\n";
        assert_eq!(parse_srt_cues(input), Err(2));
        assert_eq!(parse_srt_cues("00:00:61,000 --> 00:00:62,000\nBad seconds"), Err(1));
        let out_of_order = "1\n00:00:02,000 --> 00:00:03,000\nLater\n\n2\n00:00:01,000 --> 00:00:02,000\nEarlier";
        assert_eq!(parse_srt_cues(out_of_order), Err(2));
    }

    #[test]
    fn srt_accepts_position_fields_without_changing_timing() {
        let cues = parse_srt_cues("1\n00:00:01,250 --> 00:00:02,500 X1:1 X2:2 Y1:3 Y2:4\nCaption").unwrap();
        assert_eq!(cues, vec![(1.25, 2.5, "Caption".into())]);
        assert_eq!(parse_srt_cues("00:00:01,250 --> 00:00:02,500 X1:no\nCaption"), Err(1));
    }
}

pub(crate) fn parse_srt_transcript(text: &str) -> Result<serde_json::Value> {
    let cues = parse_srt_cues(text).map_err(|cue| {
        CoreError::InvalidArg(format!("Invalid timestamps or text in SRT cue {cue}"))
    })?;
    let segments: Vec<_> = cues.into_iter().map(|(start, end, text)| {
        serde_json::json!({"start": start, "end": end, "text": text})
    }).collect();
    Ok(serde_json::json!({"segments": segments}))
}

pub(crate) fn parse_external_transcript(document: serde_json::Value) -> Result<Transcript> {
    let input: ExternalTranscript = serde_json::from_value(document)?;
    let segments = input.segments.into_iter().map(|segment| TranscriptSegment {
        original_text: None,
        id: Uuid::new_v4(),
        start: segment.start,
        end: segment.end,
        text: segment.text,
        speaker: segment.speaker,
        words: segment.words.into_iter().map(|word| Word {
            text: word.text,
            original_text: None,
            start: word.start,
            end: word.end,
            confidence: word.confidence.unwrap_or(0.0),
        }).collect(),
        is_filler: false,
        is_silence: false,
        take_group_id: None,
        is_best_take: false,
    }).collect();
    Ok(Transcript {
        id: Uuid::new_v4(),
        media_id: Uuid::nil(),
        language: "und".into(),
        model_used: "Imported timed JSON".into(),
        source_fingerprint: None,
        source_audio_stream_index: None,
        segments,
        words_trimmed: false,
        words_aligned: false,
    })
}
