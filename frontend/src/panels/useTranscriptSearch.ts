import { useDeferredValue, useMemo, useState } from "react";
import type { Preferences, TranscriptSegment } from "../ipc/types";
import { overlapsRange, type Range } from "../playback/time";
import { FILLER_REVIEW_PHRASES, hasLowConfidence, searchTranscript } from "./transcriptSearch";

export function useTranscriptSearch(
  segments: readonly TranscriptSegment[],
  cutIds: ReadonlySet<string>,
  excludedRanges: readonly (readonly [number, number])[],
  showCuts: boolean,
  prefs: Preferences | undefined,
  highlightedRanges: readonly Range[],
) {
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWords, setWholeWords] = useState(true);
  const [matchAnyPhrase, setMatchAnyPhrase] = useState(false);
  const [searchCorrectedText, setSearchCorrectedText] = useState(false);
  const [searchStart, setSearchStart] = useState("");
  const [searchEnd, setSearchEnd] = useState("");
  const [phraseList, setPhraseList] = useState<"fillers" | "profanity" | "keywords" | null>(null);
  const [showRepeatedTakes, setShowRepeatedTakes] = useState(false);
  const [showLowConfidence, setShowLowConfidence] = useState(false);
  const [showHighlights, setShowHighlights] = useState(false);
  const filteringHighlights = showHighlights && highlightedRanges.length > 0;
  const [speaker, setSpeaker] = useState("");
  const [excludeSpeaker, setExcludeSpeaker] = useState(false);
  const speakers = useMemo(() => [...new Set(segments.flatMap((segment) => segment.speaker ? [segment.speaker] : []))].sort(), [segments]);
  const filteringSpeaker = speakers.includes(speaker);

  const usingPhraseList = phraseList !== null;
  const searching = usingPhraseList || query.trim().length > 0;
  const searchUpdating = !usingPhraseList && query !== deferredQuery;
  const hasRepeatedTakes = useMemo(() => segments.some((segment) => segment.take_group_id), [segments]);
  const filteringTakes = showRepeatedTakes && hasRepeatedTakes;
  const sourceStart = Number(searchStart);
  const sourceEnd = searchEnd === "" ? Infinity : Number(searchEnd);
  const invalidSearchWindow = !Number.isFinite(sourceStart) || sourceStart < 0 || !(sourceEnd > sourceStart);
  const fillerQueries = useMemo(
    () => [...FILLER_REVIEW_PHRASES, ...(prefs?.custom_filler_words ?? [])],
    [prefs?.custom_filler_words],
  );
  const eligibleSegments = useMemo(() => new Set(segments.filter((segment) => {
    if (!showCuts && cutIds.has(segment.id)) return false;
    if (filteringTakes && !segment.take_group_id) return false;
    if (showLowConfidence && !segment.words.some((word) => hasLowConfidence(word.confidence))) return false;
    return !filteringHighlights || overlapsRange(segment, highlightedRanges);
  }).map((segment) => segment.id)), [segments, showCuts, cutIds, filteringTakes, showLowConfidence, filteringHighlights, highlightedRanges]);
  const matches = useMemo(
    () => searchTranscript(
      segments, phraseList === "fillers" ? fillerQueries : phraseList === "profanity" ? prefs?.profanity_phrases ?? []
        : phraseList === "keywords" ? prefs?.keyword_phrases ?? [] : matchAnyPhrase ? deferredQuery.split(",") : deferredQuery,
      !usingPhraseList && caseSensitive,
      showCuts ? [] : excludedRanges, usingPhraseList || wholeWords,
      { start: sourceStart, end: sourceEnd },
      !usingPhraseList && searchCorrectedText,
      filteringSpeaker ? { name: speaker, exclude: excludeSpeaker } : undefined,
      eligibleSegments,
    ),
    [segments, deferredQuery, caseSensitive, showCuts, excludedRanges, phraseList, usingPhraseList, prefs?.profanity_phrases, prefs?.keyword_phrases, fillerQueries, wholeWords, matchAnyPhrase, sourceStart, sourceEnd, searchCorrectedText, filteringSpeaker, speaker, excludeSpeaker, eligibleSegments],
  );
  const visible = useMemo(() => {
    return segments.filter((seg) => {
      if (!eligibleSegments.has(seg.id)) return false;
      if (filteringSpeaker && (seg.speaker === speaker) === excludeSpeaker) return false;
      return !searching || matches.has(seg.id);
    });
  }, [segments, eligibleSegments, searching, matches, filteringSpeaker, speaker, excludeSpeaker]);
  const matchCount = useMemo(
    () => visible.reduce((count, segment) => count + (matches.get(segment.id)?.count ?? 0), 0),
    [visible, matches],
  );
  const untimedMatchCount = useMemo(
    () => visible.reduce((count, segment) => count + (matches.get(segment.id)?.untimed ?? 0), 0),
    [visible, matches],
  );
  const matchRanges = useMemo(
    () => visible.flatMap((segment) => matches.get(segment.id)?.ranges ?? []),
    [visible, matches],
  );

  return {
    query, setQuery, caseSensitive, setCaseSensitive, wholeWords, setWholeWords,
    matchAnyPhrase, setMatchAnyPhrase, searchStart, setSearchStart, searchEnd, setSearchEnd,
    phraseList, setPhraseList, showRepeatedTakes, setShowRepeatedTakes,
    usingPhraseList, searching, searchUpdating, hasRepeatedTakes, filteringTakes, invalidSearchWindow,
    matches, visible, matchCount, untimedMatchCount, matchRanges,
    showLowConfidence, setShowLowConfidence,
    filteringHighlights, setShowHighlights,
    searchCorrectedText, setSearchCorrectedText,
    speaker, setSpeaker, speakers, excludeSpeaker, setExcludeSpeaker, filteringSpeaker,
  };
}
