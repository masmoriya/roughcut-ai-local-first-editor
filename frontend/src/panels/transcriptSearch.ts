import type { TranscriptSegment } from "../ipc/types";
import { mergeRanges, type Range } from "../playback/time";

export function hasLowConfidence(score: number | undefined): boolean {
  return score !== undefined && score > 0 && score < 0.6;
}

export const FILLER_REVIEW_PHRASES = [
  "um", "uh", "uhm", "er", "erm", "ah", "hmm", "mhm",
  "you know", "like", "sort of", "kind of", "I mean", "okay", "so",
] as const;

function tokenize(text: string, caseSensitive: boolean): string[] {
  const normalized = text.normalize("NFKC").replace(/[’‘]/g, "'");
  return (caseSensitive ? normalized : normalized.toLowerCase())
    .match(/[\p{L}\p{M}\p{N}]+(?:'[\p{L}\p{M}\p{N}]+)*/gu) ?? [];
}

type TimedToken = { text: string; segmentId: string; index: number; start: number; end: number };
export type TimedPhraseMatch = Range & { occurrenceKey: string; ruleKey: string };
type Match = { count: number; untimed: number; words: Set<number>; ranges: TimedPhraseMatch[] };

export function searchTranscript(
  segments: readonly TranscriptSegment[], query: string | readonly string[], caseSensitive: boolean,
  hiddenRanges: readonly (readonly [number, number])[] = [],
  wholeWords = true,
  window: Range = { start: 0, end: Infinity },
  correctedTextOnly = false,
  speakerFilter?: { name: string; exclude: boolean },
  eligibleSegments?: ReadonlySet<string>,
) {
  const phrases = (typeof query === "string" ? [query] : query)
    .map((value) => tokenize(value, caseSensitive))
    .filter((phrase) => phrase.length > 0);
  const matches = new Map<string, Match>();
  if (!phrases.length || !Number.isFinite(window.start) || window.start < 0 || !(window.end > window.start)) return matches;
  const overlappingSpeech = new Set<string>();
  if (!correctedTextOnly && wholeWords && segments.some((segment) => !segment.is_silence && !segment.words.length)) {
    const speech = segments.filter((segment) => !segment.is_silence && segment.end > segment.start)
      .sort((a, b) => a.start - b.start);
    let previousEnd = -Infinity;
    for (const [index, segment] of speech.entries()) {
      const next = speech[index + 1];
      if (previousEnd > segment.start || (next && next.start < segment.end)) {
        overlappingSpeech.add(segment.id);
      }
      previousEnd = Math.max(previousEnd, segment.end);
    }
  }
  const hidden = mergeRanges(hiddenRanges.map(([start, end]) => ({ start, end })));
  const firstHiddenAfter = (time: number) => {
    let low = 0;
    let high = hidden.length;
    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if (hidden[mid]!.end <= time) low = mid + 1;
      else high = mid;
    }
    return hidden[low];
  };
  const matchesToken = (actual: string | undefined, expected: string) =>
    wholeWords ? actual === expected : actual?.includes(expected) === true;
  const ruleKey = (phrase: string[]) => JSON.stringify([caseSensitive, wholeWords, phrase]);
  const ruleKeys = new Map(phrases.map((phrase) => [phrase, ruleKey(phrase)]));
  const byFirstToken = new Map<string, string[][]>();
  for (const phrase of phrases) {
    const first = phrase[0]!;
    const indexed = byFirstToken.get(first);
    if (indexed) indexed.push(phrase);
    else byFirstToken.set(first, [phrase]);
  }
  const candidates = (first: string) => wholeWords ? byFirstToken.get(first) ?? [] : phrases;
  const entry = (id: string) => {
    const current = matches.get(id) ?? { count: 0, untimed: 0, words: new Set<number>(), ranges: [] };
    matches.set(id, current);
    return current;
  };

  const seen = new Set<string>();
  let tokens: TimedToken[] = [];
  const scan = () => {
    for (let start = 0; start < tokens.length; start++) {
      for (const phrase of candidates(tokens[start]!.text)) {
        if (!phrase.every((token, offset) => matchesToken(tokens[start + offset]?.text, token))) continue;
        const first = tokens[start]!;
        const last = tokens[start + phrase.length - 1]!;
        const rule = ruleKeys.get(phrase)!;
        const occurrenceKey = `${first.segmentId}:${first.index}:${last.segmentId}:${last.index}`;
        const key = JSON.stringify([occurrenceKey, rule]);
        if (seen.has(key)) continue;
        seen.add(key);
        let end = first.end;
        for (let index = start; index < start + phrase.length; index++) {
          const token = tokens[index]!;
          entry(token.segmentId).words.add(token.index);
          end = Math.max(end, token.end);
        }
        const result = entry(first.segmentId);
        result.count++;
        if (!correctedTextOnly && end > first.start) {
          result.ranges.push({ start: first.start, end, occurrenceKey, ruleKey: rule });
        }
        else result.untimed++;
      }
    }
    tokens = [];
  };

  let previousSpeaker: string | undefined;
  for (const segment of segments) {
    if (segment.is_silence || (eligibleSegments && !eligibleSegments.has(segment.id))) {
      scan();
      continue;
    }
    if (previousSpeaker !== segment.speaker) scan();
    previousSpeaker = segment.speaker;
    if (speakerFilter && (segment.speaker === speakerFilter.name) === speakerFilter.exclude) {
      scan();
      continue;
    }
    if (!segment.words.length) {
      scan();
      if (segment.start < window.start || segment.end > window.end ||
          (firstHiddenAfter(segment.start)?.start ?? Infinity) < segment.end) continue;
      const original = (segment as TranscriptSegment & { original_text?: string }).original_text;
      const text = tokenize(correctedTextOnly ? segment.text : original ?? segment.text, caseSensitive);
      const count = text.reduce((sum, first, index) =>
        sum + Number(candidates(first).some((phrase) => phrase.every((token, offset) => matchesToken(text[index + offset], token)))), 0);
      if (count) {
        const result = entry(segment.id);
        result.count += count;
        const matchesEntireCue = !correctedTextOnly && wholeWords && text.length > 0 &&
          candidates(text[0]!).some((phrase) =>
            phrase.length === text.length && phrase.every((token, index) => text[index] === token));
        if (matchesEntireCue && !overlappingSpeech.has(segment.id) && segment.end > segment.start) {
          result.ranges.push({ start: segment.start, end: segment.end, occurrenceKey: segment.id, ruleKey: ruleKey(text) });
          result.untimed += count - 1;
        } else {
          result.untimed += count;
        }
      }
      continue;
    }
    for (const [index, word] of segment.words.entries()) {
      if (!Number.isFinite(word.start) || !Number.isFinite(word.end) || word.end < word.start ||
          word.start < window.start || word.end > window.end ||
          (firstHiddenAfter(word.start)?.start ?? Infinity) < word.end) {
        scan();
        continue;
      }
      const previous = tokens.at(-1);
      if (previous && (word.start - previous.end > 1 || word.start < previous.start ||
          (word.start > previous.end && (firstHiddenAfter(previous.end)?.start ?? Infinity) < word.start))) scan();
      const spoken = correctedTextOnly ? word.text : (word as typeof word & { original_text?: string }).original_text ?? word.text;
      for (const text of tokenize(spoken, caseSensitive)) {
        tokens.push({ text, segmentId: segment.id, index, start: word.start, end: word.end });
      }
    }
  }
  scan();
  return matches;
}
