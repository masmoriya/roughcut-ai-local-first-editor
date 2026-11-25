import { useStore } from "@tanstack/react-store";
import type { CutSuggestion, ProposedCut, TranscriptSegment, Word } from "../ipc/types";
import type { Range } from "../playback/time";
import { seekTo, selectSegment, setFocusedCut, viewStore } from "../state/viewStore";
import { hearCut } from "./CutReview";
import { hasLowConfidence } from "./transcriptSearch";
import { clock } from "./clipUtil";

export const normalizeFillerWord = (word: string) => word.toLowerCase().replace(/[^a-z']/g, "");

function WordSpan({
  word,
  isFiller,
  highlight,
  savedHighlight,
  cutWord,
  current,
  inClip,
  proposed,
  protectedWord,
}: {
  word: Word;
  isFiller: boolean;
  highlight: boolean;
  savedHighlight: boolean;
  cutWord: boolean;
  current: boolean;
  /** Part of the clip being assembled ("Add to clip"). */
  inClip: boolean;
  /** In a proposed cut: "on", or "focus" when the review bar is on it. */
  proposed: "" | "on" | "focus";
  protectedWord: boolean;
}) {
  const lowConfidence = hasLowConfidence(word.confidence);
  const originalText = (word as Word & { original_text?: string }).original_text;
  const description = [
    savedHighlight && "Saved highlight",
    protectedWord && "Protected from automatic cuts",
    lowConfidence && "Low transcription confidence",
    originalText && originalText !== word.text && `Original transcript: ${originalText}`,
  ].filter(Boolean).join(". ");
  return (
    <span
      className={`word${isFiller ? " filler" : ""}${highlight ? " hit" : ""}${cutWord ? " cutw" : ""}${current ? " current" : ""}${inClip ? " inclip" : ""}${proposed ? " proposedw" : ""}${proposed === "focus" ? " focusw" : ""}`}
      data-wstart={word.start}
      data-wend={word.end}
      data-protected={protectedWord || undefined}
      data-saved-highlight={savedHighlight || undefined}
      data-low-confidence={lowConfidence || undefined}
      aria-description={description || undefined}
      title={description || undefined}
      onClick={(e) => {
        e.stopPropagation();
        if (window.getSelection()?.isCollapsed !== false) seekTo(word.start);
      }}
    >
      {word.text}{" "}
    </span>
  );
}

export function TranscriptSegmentBlock({
  seg,
  cut,
  selected,
  active,
  fillerSet,
  matchedWords,
  onRestore,
  excludedRanges,
  protectedRanges,
  highlightedRanges,
  showCuts,
  suggested,
  suggestionReviewPending,
  onAcceptSuggestion,
  onDismissSuggestion,
  silenceAfter,
  onRestoreRange,
  clipDraft,
  proposedCuts,
  pausesBefore,
  focusedCut,
}: {
  seg: TranscriptSegment;
  cut: boolean;
  selected: boolean;
  active: boolean;
  fillerSet: Set<string>;
  matchedWords?: ReadonlySet<number>;
  onRestore: (id: string) => void;
  excludedRanges: Array<[number, number]>;
  protectedRanges: readonly Range[];
  highlightedRanges: readonly Range[];
  showCuts: boolean;
  suggested?: CutSuggestion;
  suggestionReviewPending: boolean;
  onAcceptSuggestion: (id: string) => void;
  onDismissSuggestion: (id: string) => void;
  silenceAfter?: { start: number; end: number; dur: number };
  onRestoreRange: (start: number, end: number) => void;
  clipDraft: Range[];
  /** Proposed cuts (not skipped) that touch this segment. */
  proposedCuts: ProposedCut[];
  /** Proposed pause cuts, by the index of the word they come before (-1:
   *  after the last word). */
  pausesBefore?: Map<number, ProposedCut[]>;
  focusedCut: number | null;
}) {
  // Word under the playhead — selector returns an index, so this block only
  // re-renders when the spoken word changes (and only while it's active).
  const currentWordIdx = useStore(viewStore, (s) => {
    if (!active) return -1;
    for (let i = 0; i < seg.words.length; i++) {
      const w = seg.words[i]!;
      if (s.playhead >= w.start && s.playhead < w.end) return i;
    }
    return -1;
  });

  const pauseChips = (cuts: ProposedCut[] | undefined) =>
    cuts?.map((c) => (
      <button
        key={`p${c.id}`}
        className={`proposed-pause${c.id === focusedCut ? " focus" : ""}`}
        title="A pause the proposal would cut — click to hear it"
        onClick={(e) => {
          e.stopPropagation();
          setFocusedCut(c.id);
          hearCut(c);
        }}
      >
        ⏸ {(c.end - c.start).toFixed(1)}s
      </button>
    ));
  const proposedAt = (mid: number) => proposedCuts.find((c) => mid >= c.start && mid < c.end);

  if (seg.is_silence) {
    const p = proposedAt((seg.start + seg.end) / 2);
    return (
      <div
        className={`segment silence${cut ? " cut" : ""}${selected ? " selected" : ""}${p ? " proposed" : ""}`}
        data-segid={seg.id}
        onClick={(e) => selectSegment(seg.id, e.metaKey || e.ctrlKey)}
      >
        <span className="silence-chip">
          silence · {(seg.end - seg.start).toFixed(1)}s
        </span>
        {cut && (
          <button
            className="restore-btn"
            onClick={(e) => {
              e.stopPropagation();
              onRestore(seg.id);
            }}
          >
            Restore
          </button>
        )}
      </div>
    );
  }

  const originalSegmentText = (seg as TranscriptSegment & { original_text?: string }).original_text;
  const originalDescription = seg.words.length === 0 && originalSegmentText && originalSegmentText !== seg.text
    ? `Original transcript: ${originalSegmentText}` : undefined;
  return (
    <div
      className={`segment${cut ? " cut" : ""}${selected ? " selected" : ""}${active ? " active" : ""}${suggested ? " suggested" : ""}`}
      data-segid={seg.id}
      aria-current={active ? "time" : undefined}
      tabIndex={seg.words.length === 0 ? 0 : undefined}
      role={seg.words.length === 0 ? "group" : undefined}
      aria-label={seg.words.length === 0 ? `${selected ? "Selected passage" : "Passage"} at ${clock(seg.start)}` : undefined}
      aria-description={originalDescription}
      title={originalDescription}
      aria-keyshortcuts={seg.words.length === 0 ? "Enter" : undefined}
      onKeyDown={(event) => {
        if (seg.words.length || event.target !== event.currentTarget || event.key !== "Enter") return;
        event.preventDefault();
        selectSegment(seg.id, event.metaKey || event.ctrlKey);
      }}
      onClick={(e) => selectSegment(seg.id, e.metaKey || e.ctrlKey)}
    >
      {suggested && !cut && (
        <div className="suggest-ribbon" onClick={(e) => e.stopPropagation()}>
          <span className="suggest-tag">
            {suggested.reason === "repeated_take" ? "repeated take" : "repetitive"}
          </span>
          <span className="suggest-hint">suggested cut</span>
          <button className="suggest-cut" disabled={suggestionReviewPending} onClick={() => onAcceptSuggestion(suggested.id)}>
            Review cut
          </button>
          <button className="suggest-keep" onClick={() => onDismissSuggestion(suggested.id)}>
            keep
          </button>
        </div>
      )}
      <p className="segment-text">
        {seg.speaker && <strong className="segment-speaker">{seg.speaker} </strong>}
        {seg.words.length === 0 && (matchedWords || highlightedRanges.some((range) => range.start < seg.end && seg.start < range.end)
          ? <mark>{seg.text}</mark> : seg.text)}
        {seg.words.length === 0 && (
          <button type="button" className="link-btn" aria-label={`Seek to ${clock(seg.start)}`}
            onClick={(event) => { event.stopPropagation(); seekTo(seg.start); }}>
            {clock(seg.start)}
          </button>
        )}
        {seg.words.map((w, i) => {
          const mid = (w.start + w.end) / 2;
          const wordCut = !cut && excludedRanges.some(([a, b]) => mid >= a && mid < b);
          if (wordCut && !showCuts) return null;
          const p = proposedAt(mid);
          return (
            <span key={i}>
              {pauseChips(pausesBefore?.get(i))}
              <WordSpan
                word={w}
                isFiller={fillerSet.has(normalizeFillerWord((w as Word & { original_text?: string }).original_text ?? w.text))}
                highlight={matchedWords?.has(i) ?? false}
                savedHighlight={highlightedRanges.some((range) => range.start < w.end && w.start < range.end)}
                protectedWord={protectedRanges.some((range) => range.start < w.end && w.start < range.end)}
                cutWord={wordCut}
                current={i === currentWordIdx}
                inClip={clipDraft.some((r) => mid >= r.start && mid < r.end)}
                proposed={p ? (p.id === focusedCut ? "focus" : "on") : ""}
              />
            </span>
          );
        })}
        {pauseChips(pausesBefore?.get(-1))}
      </p>
      {cut && (
        <button
          className="restore-btn"
          onClick={(e) => {
            e.stopPropagation();
            onRestore(seg.id);
          }}
        >
          Restore
        </button>
      )}
      {silenceAfter && (
        <button
          className="silence-removed"
          title="A silent gap was removed here — click to restore it"
          onClick={(e) => {
            e.stopPropagation();
            onRestoreRange(silenceAfter.start, silenceAfter.end);
          }}
        >
          ⏸ {silenceAfter.dur.toFixed(1)}s pause removed · restore
        </button>
      )}
    </div>
  );
}
