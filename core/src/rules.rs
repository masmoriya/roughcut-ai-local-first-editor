use crate::model::Transcript;

fn phrase_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    let normalized: String = text.nfkc().flat_map(char::to_lowercase).map(|character| {
        match character {
            '’' | '‘' => '\'',
            value => value,
        }
    }).collect();
    normalized.split(|character: char| !character.is_alphanumeric() && !is_combining_mark(character) && character != '\'')
        .map(|token| token.trim_matches('\''))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>().into_iter()
}

#[cfg(test)]
mod phrase_token_tests {
    use super::phrase_tokens;

    #[test]
    fn normalizes_accents_and_curly_apostrophes() {
        assert_eq!(phrase_tokens("CAFE\u{301} — ‘don’t’").collect::<Vec<_>>(), ["café", "don't"]);
    }

    #[test]
    fn keeps_words_separate_without_matching_substrings() {
        assert_eq!(
            phrase_tokens("number umbrella um—like-minded").collect::<Vec<_>>(),
            ["number", "umbrella", "um", "like", "minded"],
        );
    }
}

pub fn never_cut_ranges(transcript: Option<&Transcript>, phrases: &[String]) -> Vec<crate::model::SourceRange> {
    let Some(transcript) = transcript.filter(|_| !phrases.is_empty()) else { return Vec::new(); };
    let mut by_first_token = std::collections::HashMap::<String, Vec<Vec<String>>>::new();
    for phrase in phrases {
        let tokens: Vec<_> = phrase_tokens(phrase).collect();
        if let Some(first) = tokens.first() {
            by_first_token.entry(first.clone()).or_default().push(tokens);
        }
    }
    let mut ranges = Vec::new();
    let mut timed = Vec::<(String, f64, f64)>::new();
    let scan = |words: &mut Vec<(String, f64, f64)>, ranges: &mut Vec<crate::model::SourceRange>| {
        for (index, first) in words.iter().enumerate() {
            let Some(phrases) = by_first_token.get(&first.0) else { continue; };
            for phrase in phrases {
                let Some(window) = words.get(index..index + phrase.len()) else { continue; };
                if window.iter().map(|word| &word.0).eq(phrase.iter()) {
                    ranges.push(crate::model::SourceRange {
                        start: window[0].1,
                        end: window.iter().map(|word| word.2).fold(window[0].2, f64::max),
                    });
                }
            }
        }
        words.clear();
    };
    let mut previous_speaker = None;
    for segment in &transcript.segments {
        if segment.is_silence || previous_speaker != segment.speaker.as_deref() {
            scan(&mut timed, &mut ranges);
        }
        previous_speaker = segment.speaker.as_deref();
        if segment.is_silence {
            continue;
        }
        if segment.words.is_empty() {
            if !segment.start.is_finite() || !segment.end.is_finite() || segment.end < segment.start {
                scan(&mut timed, &mut ranges);
                continue;
            }
            if timed.last().is_some_and(|previous| segment.start < previous.1 || segment.start - previous.2 > 1.0) {
                scan(&mut timed, &mut ranges);
            }
            let text = segment.original_text.as_deref().unwrap_or(&segment.text);
            timed.extend(phrase_tokens(text).map(|token| (token, segment.start, segment.end)));
            continue;
        }
        for word in &segment.words {
            if !word.start.is_finite() || !word.end.is_finite() || word.end < word.start {
                scan(&mut timed, &mut ranges);
                continue;
            }
            if timed.last().is_some_and(|previous| word.start < previous.1 || word.start - previous.2 > 1.0) {
                scan(&mut timed, &mut ranges);
            }
            timed.extend(phrase_tokens(word.spoken_text()).map(|text| (text, word.start, word.end)));
        }
    }
    scan(&mut timed, &mut ranges);
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{parse_external_transcript, parse_srt_transcript};

    #[test]
    fn never_cut_phrase_spans_adjacent_cues_but_stops_at_long_gaps() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nKeep\n\n\
                   2\n00:00:02,100 --> 00:00:03,000\nthis phrase\n\n\
                   3\n00:00:05,000 --> 00:00:06,000\nKeep\n\n\
                   4\n00:00:08,000 --> 00:00:09,000\nthis phrase\n";
        let transcript = parse_external_transcript(parse_srt_transcript(srt).unwrap()).unwrap();
        let ranges = never_cut_ranges(Some(&transcript), &["keep this phrase".into()]);
        assert_eq!(ranges.len(), 1);
        assert_eq!((ranges[0].start, ranges[0].end), (1.0, 3.0));
    }
}
