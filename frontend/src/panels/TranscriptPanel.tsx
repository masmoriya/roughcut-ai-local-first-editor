// Script panel: virtualized transcript as flowing paragraphs. Words seek the
// playhead; selected segments can be cut with Delete/Backspace; cut segments
// show struck-through (with a restore button) when "Show cuts" is on.

import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useStore } from "@tanstack/react-store";
import {
  useAcceptAllSuggestions,
  useAcceptSuggestion,
  useCutByTranscript,
  useCutRange,
  useDismissSuggestion,
  useGenerateRoughCut,
  usePreferences,
  useProject,
  useProtectSourceRanges,
  useHighlightSourceRanges,
  useRestoreByTranscript,
  useRestoreRange,
  useReviewPauses,
  useSuggestions,
  useTimeline,
  useTranscript,
} from "../ipc/queries";
import {
  addToClipDraft,
  removeFromClipDraft,
  seekTo,
  setSelectedSegmentIds,
  viewStore,
} from "../state/viewStore";
import { CutReview } from "./CutReview";
import { CustomFillers } from "./CustomFillers";
import { SavedRuleSets } from "./SavedRuleSets";
import { TranscriptSearchActions } from "./TranscriptSearchActions";
import { TranscriptSourceActions } from "./TranscriptSourceActions";
import { overlapsRange, type Range } from "../playback/time";
import { ClipDraftBar } from "./ClipDraftBar";
import { onAppEvent } from "../ipc/api";
import type { CutSuggestion, ProgressEvent, ProposedCut, TranscriptSegment } from "../ipc/types";
import { errText } from "./clipUtil";
import { selectBetweenWords, selectedTranscriptWords } from "./transcriptSelection";
import { TranscriptCorrection } from "./TranscriptCorrection";
import { useTranscriptSearch } from "./useTranscriptSearch";
import { indexTranscriptPlayback } from "./transcriptPlaybackIndex";
import { normalizeFillerWord, TranscriptSegmentBlock } from "./TranscriptSegmentBlock";

const NO_CUTS: ProposedCut[] = [];
const NO_PROTECTED_RANGES: readonly Range[] = [];

export function TranscriptPanel({ projectId }: { projectId: string }) {
  const { data: transcriptData, error: transcriptLoadError, refetch: refetchTranscript } = useTranscript(projectId);
  const transcript = transcriptLoadError ? undefined : transcriptData;
  const { data: timeline } = useTimeline(projectId);
  const { data: prefs } = usePreferences();
  const cutByTranscript = useCutByTranscript();
  const restoreByTranscript = useRestoreByTranscript();
  const roughCut = useGenerateRoughCut();
  const { data: project } = useProject(projectId);
  const proposal = project?.cut_proposal ?? null;
  const focusedCut = useStore(viewStore, (s) => s.focusedCut);
  // The rough cut's steps, as the core reports them.
  const [roughStep, setRoughStep] = useState<string | null>(null);
  useEffect(
    () =>
      onAppEvent<ProgressEvent>("progress", (p) => {
        if (p.task !== "rough_cut" || (p.project_id && p.project_id !== projectId)) return;
        setRoughStep(p.fraction >= 1 ? null : p.message);
      }),
    [projectId],
  );
  const { data: suggestions } = useSuggestions(projectId);
  const acceptSuggestion = useAcceptSuggestion();
  const acceptAll = useAcceptAllSuggestions();
  const dismissSuggestion = useDismissSuggestion();
  // segment_id → suggestion, for inline highlighting.
  const suggestionBySeg = useMemo(() => {
    const m = new Map<string, CutSuggestion>();
    for (const s of suggestions ?? []) for (const sid of s.segment_ids) m.set(sid, s);
    return m;
  }, [suggestions]);
  const suggestionReviewFeedback = {
    onSuccess: ({ proposed }: { proposed: number }) => setRoughCutResult(
      proposed ? `${proposed} cuts ready for review` : "No new cuts to review",
    ),
    onError: (error: unknown) => setRoughCutResult(errText(error)),
  };
  const suggestionReviewPending = acceptSuggestion.isPending || acceptAll.isPending;
  const onAcceptSuggestion = (id: string) => {
    if (suggestionReviewPending) return;
    acceptSuggestion.mutate({ project_id: projectId, suggestion_id: id }, suggestionReviewFeedback);
  };
  const onDismissSuggestion = (id: string) =>
    dismissSuggestion.mutate({ project_id: projectId, suggestion_id: id });

  const showCuts = useStore(viewStore, (s) => s.showCuts);
  const clipDraft = useStore(viewStore, (s) => s.clipDraft);
  const selectedIds = useStore(viewStore, (s) => s.selectedSegmentIds);

  const [roughCutResult, setRoughCutResult] = useState<string | null>(null);
  const parentRef = useRef<HTMLDivElement | null>(null);
  const lastTranscriptId = useRef<string>();
  useEffect(() => {
    const id = transcript?.id;
    if (!id) return;
    const previous = lastTranscriptId.current;
    lastTranscriptId.current = id;
    if (!previous || previous === id) return;
    setSelectedSegmentIds([]);
    const selection = window.getSelection();
    if (selection && parentRef.current?.contains(selection.anchorNode)) selection.removeAllRanges();
  }, [transcript?.id]);

  const segments = useMemo(() => transcript?.segments ?? [], [transcript]);
  const hasWordTimings = useMemo(() => segments.some((segment) => segment.words.length > 0), [segments]);

  // A segment is cut when (almost) none of it plays. Not "linked to a cut
  // clip": a cut that was partly restored, or a saved clip's own timeline
  // (everything outside it cut), can link a segment that still mostly plays,
  // which then showed as cut, or vanished with cuts hidden.
  const cutIds = useMemo(() => {
    const ids = new Set<string>();
    const plays = (timeline?.clips ?? [])
      .filter((c) => c.included)
      .map((c) => [c.source_in, c.source_out] as const)
      .sort((a, b) => a[0] - b[0]);
    let k = 0;
    for (const seg of segments) {
      while (k > 0 && plays[k - 1]![1] > seg.start) k--;
      while (k < plays.length && plays[k]![1] <= seg.start) k++;
      let kept = 0;
      for (let j = k; j < plays.length && plays[j]![0] < seg.end; j++) {
        kept += Math.min(seg.end, plays[j]![1]) - Math.max(seg.start, plays[j]![0]);
      }
      if (kept < Math.max(0.05, 0.02 * (seg.end - seg.start))) ids.add(seg.id);
    }
    return ids;
  }, [timeline, segments]);

  // Excluded source ranges, for word-accurate strikethrough (range cuts made
  // via right-click don't link whole segments).
  const excludedRanges = useMemo(
    () =>
      (timeline?.clips ?? [])
        .filter((c) => !c.included)
        .map((c) => [c.source_in, c.source_out] as [number, number]),
    [timeline],
  );

  // Removed dead-air between segments: a gap (≥0.3s) with NO segment in it
  // that the timeline excludes was a removed pause. Marked after the earlier
  // segment, restorable — a breadcrumb shown even with "Show cuts" off.
  // (Gaps whisper emitted as is_silence segments render their own chip, so
  // iterating ALL segments in source order avoids double-marking.)
  const silenceAfter = useMemo(() => {
    const m = new Map<string, { start: number; end: number; dur: number }>();
    const segs = [...(transcript?.segments ?? [])].sort((a, b) => a.start - b.start);
    const clips = timeline?.clips ?? [];
    const includedAt = (t: number) =>
      clips.some((c) => c.included && c.source_in <= t && t < c.source_out);
    for (let i = 0; i < segs.length - 1; i++) {
      const a = segs[i]!;
      const b = segs[i + 1]!;
      const gap = b.start - a.end;
      if (gap >= 0.3 && !includedAt((a.end + b.start) / 2)) {
        m.set(a.id, { start: a.end, end: b.start, dur: gap });
      }
    }
    return m;
  }, [transcript, timeline]);

  const protectedRanges = project?.protected_ranges ?? NO_PROTECTED_RANGES;
  const highlightedRanges = (project as (typeof project & { highlighted_ranges?: Range[] }))?.highlighted_ranges ?? NO_PROTECTED_RANGES;
  // Proposed cuts (not skipped), per segment they touch; and proposed pause
  // cuts placed before the word that follows them (or after a segment's
  // last word when nothing follows), so each shows as a chip in the text.
  const { proposedBySeg, pausesBySeg } = useMemo(() => {
    const bySeg = new Map<string, ProposedCut[]>();
    const pauses = new Map<string, Map<number, ProposedCut[]>>();
    const live = (proposal?.cuts ?? []).filter((c) => !c.skipped && !overlapsRange(c, protectedRanges));
    if (live.length === 0) return { proposedBySeg: bySeg, pausesBySeg: pauses };
    const segs = [...segments].sort((a, b) => a.start - b.start);
    for (const seg of segs) {
      const touching = live.filter((c) => c.end > seg.start && c.start < seg.end);
      if (touching.length) bySeg.set(seg.id, touching);
    }
    const words = segs
      .filter((s) => !s.is_silence)
      .flatMap((s) => s.words.map((w, i) => ({ seg: s.id, i, start: w.start })));
    let wi = 0;
    for (const c of live.filter((c) => c.kind === "pause").sort((a, b) => a.start - b.start)) {
      while (wi < words.length && words[wi]!.start < c.end - 0.25) wi++;
      const at = words[wi] ?? (words.length ? { ...words[words.length - 1]!, i: -1 } : null);
      if (!at) continue;
      const m = pauses.get(at.seg) ?? new Map<number, ProposedCut[]>();
      m.set(at.i, [...(m.get(at.i) ?? []), c]);
      pauses.set(at.seg, m);
    }
    return { proposedBySeg: bySeg, pausesBySeg: pauses };
  }, [proposal, segments, protectedRanges]);

  const fillerSet = useMemo(() => {
    const s = new Set(["um", "uh"]);
    for (const w of prefs?.custom_filler_words ?? []) {
      const n = normalizeFillerWord(w);
      if (n) s.add(n);
    }
    return s;
  }, [prefs]);

  const {
    query, setQuery, caseSensitive, setCaseSensitive, wholeWords, setWholeWords,
    matchAnyPhrase, setMatchAnyPhrase, searchStart, setSearchStart, searchEnd, setSearchEnd,
    phraseList, setPhraseList, showRepeatedTakes, setShowRepeatedTakes,
    usingPhraseList, searching, searchUpdating, hasRepeatedTakes, filteringTakes, invalidSearchWindow,
    matches, visible, matchCount, untimedMatchCount, matchRanges, showLowConfidence, setShowLowConfidence,
    searchCorrectedText, setSearchCorrectedText,
    speaker, setSpeaker, speakers, excludeSpeaker, setExcludeSpeaker, filteringSpeaker,
    filteringHighlights, setShowHighlights,
  } = useTranscriptSearch(segments, cutIds, excludedRanges, showCuts, prefs, highlightedRanges);
  const reviewPauses = useReviewPauses();
  const protectSourceRanges = useProtectSourceRanges();
  const highlightSourceRanges = useHighlightSourceRanges();
  const protectedCount = protectedRanges.length;

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    overscan: 10,
  });

  // Track the segment under the playhead (re-renders only when the id changes).
  const activeSegmentAt = useMemo(() => indexTranscriptPlayback(segments), [segments]);
  const activeSegId = useStore(viewStore, (s) => activeSegmentAt(s.playhead));
  const visibleIndex = useMemo(() => new Map(visible.map((segment, index) => [segment.id, index])), [visible]);

  // Follow the playhead: scroll the active segment into view while scrubbing
  // or playing — but yield to the user's own scrolling for a moment.
  const userScrollAtRef = useRef(0);
  const selectingText = useRef(false);
  const wordAnchor = useRef<HTMLElement | null>(null);
  const keyboardWordEnd = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!activeSegId) return;
    const selection = window.getSelection();
    if (selectingText.current || (
      selection && !selection.isCollapsed && parentRef.current?.contains(selection.anchorNode)
    )) return;
    if (Date.now() - userScrollAtRef.current < 2500) return;
    const idx = visibleIndex.get(activeSegId);
    if (idx !== undefined) virtualizer.scrollToIndex(idx, { align: "auto" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSegId]);

  const cutRange = useCutRange();
  const restoreRange = useRestoreRange();
  const [selectionError, setSelectionError] = useState("");

  // A text selection takes precedence over a paragraph selection.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat || e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target;
      if (!(target instanceof HTMLElement) || !parentRef.current?.contains(target) || target.isContentEditable ||
          target.closest("input, textarea, select, button, a[href], [role='button']")) return;
      if (e.key === "Delete" || e.key === "Backspace") {
        if (!transcript || searchUpdating || cutRange.isPending || cutByTranscript.isPending) return;
        const range = selectedTranscriptWords(parentRef.current);
        if (range) {
          e.preventDefault();
          if ((searching || filteringTakes || showLowConfidence || filteringSpeaker || filteringHighlights) && range.crossesParagraphs) {
            setSelectionError("Clear the search before cutting across paragraphs, or use Review cuts for the matches.");
            return;
          }
          setSelectionError("");
          cutRange.mutate({ project_id: projectId, transcript_id: transcript.id, start: range.start, end: range.end }, {
            onSuccess: () => {
              window.getSelection()?.removeAllRanges();
              setSelectedSegmentIds([]);
            },
            onError: (error) => setSelectionError(errText(error)),
          });
          return;
        }
        if (window.getSelection()?.isCollapsed === false || !viewStore.state.selectedSegmentIds.length) return;
        e.preventDefault();
        const selectedIds = viewStore.state.selectedSegmentIds;
        if (selectedIds.some((id) => !visibleIndex.has(id))) {
          setSelectionError("Clear the filters or select visible passages before cutting.");
          return;
        }
        setSelectionError("");
        cutByTranscript.mutate({
          project_id: projectId,
          transcript_id: transcript.id,
          segment_ids: selectedIds,
        }, {
          onSuccess: () => setSelectedSegmentIds([]),
          onError: (error) => setSelectionError(errText(error)),
        });
      } else if (e.key === "Escape") {
        const selection = window.getSelection();
        if (selection && parentRef.current?.contains(selection.anchorNode)) selection.removeAllRanges();
        setSelectedSegmentIds([]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId, transcript, cutRange.isPending, cutByTranscript.isPending, searching, filteringTakes, searchUpdating, showLowConfidence, filteringSpeaker, filteringHighlights, visibleIndex]);

  useEffect(() => {
    const endSelection = () => { selectingText.current = false; };
    window.addEventListener("pointerup", endSelection);
    window.addEventListener("pointercancel", endSelection);
    window.addEventListener("blur", endSelection);
    return () => {
      window.removeEventListener("pointerup", endSelection);
      window.removeEventListener("pointercancel", endSelection);
      window.removeEventListener("blur", endSelection);
    };
  }, []);

  const onRestore = (id: string) => {
    restoreByTranscript.mutate({ project_id: projectId, segment_ids: [id] });
  };
  const onRestoreRange = (start: number, end: number) => {
    restoreRange.mutate({ project_id: projectId, start, end });
  };

  // ---- right-click context menu: cut/restore the selected text's video ----
  const [selectionMenu, setMenu] = useState<{
    transcriptId: string;
    x: number;
    y: number;
    label: string;
    text: string;
    range: { start: number; end: number } | null;
    segIds: string[];
    correctionSegmentId?: string;
    keyboard: boolean;
  } | null>(null);
  const menu = selectionMenu?.transcriptId === transcript?.id ? selectionMenu : null;
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const contextMenuOrigin = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (menu?.keyboard) contextMenuRef.current?.querySelector("button")?.focus();
  }, [menu]);
  const [correction, setCorrection] = useState<TranscriptSegment | null>(null);
  const correctionTarget = menu?.correctionSegmentId
    ? segments.find((segment) => segment.id === menu.correctionSegmentId && !segment.is_silence)
    : undefined;

  const openContextMenu = (target: HTMLElement, x: number, y: number, keyboard = false) => {
    if (!transcript) return;
    // 1st choice: the actual text selection, mapped to a source time range
    // via the word spans' data attributes.
    const range = selectedTranscriptWords(parentRef.current);
    if ((searching || filteringTakes || showLowConfidence || filteringSpeaker || filteringHighlights) && range?.crossesParagraphs) {
      setMenu(null);
      setSelectionError("Clear the search before editing across paragraphs, or use Review cuts for the matches.");
      return;
    }
    setSelectionError("");
    // Fallback: the clicked paragraph (plus any selected paragraphs).
    let segIds: string[] = [];
    if (!range) {
      const segEl = target.closest<HTMLElement>("[data-segid]");
      const segId = segEl?.dataset.segid;
      if (segId) segIds = selectedIds.includes(segId) ? selectedIds : [segId];
      else if (selectedIds.length > 0) segIds = selectedIds;
    }
    if (!range && segIds.length === 0) return;
    if (segIds.some((id) => !visibleIndex.has(id))) {
      setMenu(null);
      setSelectionError("Clear the filters or select visible passages before editing.");
      return;
    }
    const label = range
      ? `${range.count} word${range.count === 1 ? "" : "s"} · ${(range.end - range.start).toFixed(1)}s`
      : `${segIds.length} paragraph${segIds.length === 1 ? "" : "s"}`;
    contextMenuOrigin.current = keyboard ? target : null;
    setMenu({
      transcriptId: transcript.id,
      x: Math.max(0, Math.min(x, window.innerWidth - 220)),
      y: Math.max(0, Math.min(y, window.innerHeight - 230)),
      keyboard,
      label,
      text: range
        ? window.getSelection()?.toString().trim() ?? ""
        : segments.filter((segment) => segIds.includes(segment.id) && !segment.is_silence)
            .map((segment) => segment.text.trim()).filter(Boolean).join("\n"),
      range,
      segIds,
      correctionSegmentId: range?.segmentId ?? (segIds.length === 1 ? segIds[0] : undefined),
    });
  };

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      close();
      if (menu.keyboard) {
        const origin = contextMenuOrigin.current;
        (origin?.isConnected ? origin : parentRef.current)?.focus({ preventScroll: true });
      }
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", close, true);
    };
  }, [menu]);

  const finishMenuAction = () => {
    window.getSelection()?.removeAllRanges();
    setSelectedSegmentIds([]);
    setMenu(null);
    if (menu?.keyboard) parentRef.current?.focus();
  };

  const menuCut = () => {
    if (!menu || searchUpdating || cutRange.isPending || cutByTranscript.isPending) return;
    setSelectionError("");
    const callbacks = {
      onSuccess: finishMenuAction,
      onError: (error: unknown) => setSelectionError(errText(error)),
    };
    if (menu.range) {
      cutRange.mutate({ project_id: projectId, transcript_id: menu.transcriptId, start: menu.range.start, end: menu.range.end }, callbacks);
    } else if (menu.segIds.length > 0) {
      cutByTranscript.mutate({ project_id: projectId, transcript_id: menu.transcriptId, segment_ids: menu.segIds }, callbacks);
    }
  };

  const menuRestore = () => {
    if (!menu || !showCuts) return;
    const callbacks = {
      onSuccess: finishMenuAction,
      onError: (error: unknown) => setSelectionError(errText(error)),
    };
    if (menu.range) {
      restoreRange.mutate({ project_id: projectId, transcript_id: menu.transcriptId, start: menu.range.start, end: menu.range.end }, callbacks);
    } else if (menu.segIds.length > 0) {
      restoreByTranscript.mutate({ project_id: projectId, transcript_id: menu.transcriptId, segment_ids: menu.segIds }, callbacks);
    }
  };

  // The menu's target as clip pieces: the text selection, or each selected
  // paragraph as its own piece (gaps between them are skipped in the clip).
  const menuPieces = (): Range[] => {
    if (!menu) return [];
    if (menu.range) return [menu.range];
    return menu.segIds
      .map((id) => segments.find((s) => s.id === id))
      .filter((s): s is TranscriptSegment => !!s)
      .map((s) => ({ start: s.start, end: s.end }));
  };
  const menuRanges = menuPieces();
  const menuInClip = menuRanges.some((p) => clipDraft.some((r) => r.start < p.end && p.start < r.end));
  const menuIsHighlighted = menuRanges.length > 0 && menuRanges.every((piece) =>
    highlightedRanges.some((range) => range.start <= piece.start && piece.end <= range.end));

  const menuAddToClip = () => {
    menuPieces().forEach(addToClipDraft);
    finishMenuAction();
  };

  const menuRemoveFromClip = () => {
    menuPieces().forEach(removeFromClipDraft);
    finishMenuAction();
  };

  const menuPlayHere = () => {
    if (!menu) return;
    const t = menu.range?.start ?? segments.find((s) => s.id === menu.segIds[0])?.start;
    if (t != null) seekTo(t);
    setMenu(null);
  };

  const onRoughCut = () => {
    setRoughCutResult(null);
    roughCut.mutate(
      { project_id: projectId, aggressiveness: prefs?.cut_aggressiveness ?? "natural" },
      {
        onSuccess: (res) => {
          setRoughStep(null);
          setRoughCutResult(
            res.cut_count === 0
              ? "nothing to cut: no pauses or filler words found"
              : `${res.cut_count} cuts proposed — nothing is cut yet; review them below`,
          );
        },
        onError: (err) => {
          setRoughStep(null);
          setRoughCutResult(`rough cut failed: ${errText(err)}`);
        },
      },
    );
  };

  return (
    <div className="transcript-panel">
      <div className="transcript-toolbar">
        <div className="search-box">
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6">
            <circle cx="7" cy="7" r="5" />
            <path d="M11 11l3.5 3.5" />
          </svg>
          <input
            type="search"
            placeholder={matchAnyPhrase && !usingPhraseList ? "Phrases separated by commas" : "Search transcript"}
            aria-label="Search transcript"
            value={query}
            onChange={(e) => {
              setPhraseList(null);
              setQuery(e.target.value);
            }}
          />
        </div>
        {([ ["fillers", "Fillers"], ["profanity", "Profanity"], ["keywords", "Keywords"] ] as const).map(([kind, label]) => (
          <button key={kind} type="button" className="ghost-btn" aria-pressed={phraseList === kind}
            onClick={() => setPhraseList(phraseList === kind ? null : kind)}>{label}</button>
        ))}
        {phraseList && prefs && <CustomFillers key={phraseList} kind={phraseList}
          phrases={phraseList === "fillers" ? prefs.custom_filler_words : phraseList === "profanity" ? prefs.profanity_phrases ?? [] : prefs.keyword_phrases ?? []} />}
        {phraseList && prefs && <CustomFillers kind="exceptions" phrases={prefs.never_cut_phrases ?? []} />}
        {phraseList && prefs && <SavedRuleSets preferences={prefs} />}
        {hasRepeatedTakes && (
          <button type="button" className="ghost-btn" aria-pressed={showRepeatedTakes} onClick={() => setShowRepeatedTakes(!showRepeatedTakes)}>
            Repeated takes
          </button>
        )}
        {transcript && <button type="button" className="ghost-btn" aria-pressed={showLowConfidence}
          onClick={() => setShowLowConfidence(!showLowConfidence)}>Low confidence</button>}
        {highlightedRanges.length > 0 && (
          <button type="button" className="ghost-btn" aria-pressed={filteringHighlights}
            onClick={() => setShowHighlights(!filteringHighlights)}>Highlights only</button>
        )}
        {highlightedRanges.length > 0 && (
          <button type="button" className="ghost-btn" onClick={() => {
            const { playhead, skipCuts } = viewStore.state;
            const starts = highlightedRanges.flatMap((range) => {
              if (!skipCuts) return [range.start];
              const clip = timeline?.clips.find((clip) => clip.included
                && clip.source_in < range.end && range.start < clip.source_out);
              return clip ? [Math.max(range.start, clip.source_in)] : [];
            });
            const next = starts.find((start) => start > playhead + 0.001) ?? starts[0];
            if (next === undefined) {
              setSelectionError("Switch to Original preview to hear the saved highlights.");
              return;
            }
            setSelectionError("");
            seekTo(next);
          }}>Next highlight</button>
        )}
        {speakers.length > 0 && <>
          <label>Speaker <select value={filteringSpeaker ? speaker : ""} onChange={(event) => setSpeaker(event.target.value)}>
            <option value="">All speakers</option>
            {speakers.map((name) => <option key={name} value={name}>{name}</option>)}
          </select></label>
          {filteringSpeaker && <label><input type="checkbox" checked={excludeSpeaker}
            onChange={(event) => setExcludeSpeaker(event.target.checked)} /> Exclude speaker</label>}
        </>}
        {protectedCount > 0 && (
          <button className="ghost-btn" disabled={protectSourceRanges.isPending}
            onClick={() => protectSourceRanges.mutate({ project_id: projectId, ranges: [] }, {
              onError: (error) => setSelectionError(errText(error)),
            })}>
            Clear protection ({protectedCount})
          </button>
        )}
        {searching && (
          <>
            {!usingPhraseList && <label><input type="checkbox" checked={caseSensitive} onChange={(event) => setCaseSensitive(event.target.checked)} /> Match case</label>}
            {!usingPhraseList && <label><input type="checkbox" checked={wholeWords} onChange={(event) => setWholeWords(event.target.checked)} /> Whole words</label>}
            {!usingPhraseList && <label><input type="checkbox" checked={matchAnyPhrase} onChange={(event) => setMatchAnyPhrase(event.target.checked)} /> Match any comma-separated phrase</label>}
            {!usingPhraseList && <label><input type="checkbox" checked={searchCorrectedText} onChange={(event) => setSearchCorrectedText(event.target.checked)} /> Search corrected text only</label>}
            <details>
              <summary>Source time</summary>
              <label>From (seconds) <input type="number" min="0" step="any" value={searchStart} onChange={(event) => setSearchStart(event.target.value)} /></label>
              <label>To (seconds) <input type="number" min="0" step="any" value={searchEnd} onChange={(event) => setSearchEnd(event.target.value)} /></label>
            </details>
            <span role="status">{invalidSearchWindow
              ? "Enter a non-negative start and an end after it."
              : searchUpdating ? "Searching…"
              : `${matchCount} ${matchCount === 1 ? "match" : "matches"}${untimedMatchCount ? ` (${untimedMatchCount} text-only)` : ""}`}</span>
          </>
        )}
        {transcript && <TranscriptSearchActions projectId={projectId} transcriptId={transcript.id} ranges={matchRanges} category={phraseList ?? "search"}
          visible={searching && !invalidSearchWindow && matchRanges.length > 0}
          updating={searchUpdating} onResult={setRoughCutResult} />}
        <button type="button" className="ghost-btn" disabled={!transcript || reviewPauses.isPending || roughCut.isPending}
          onClick={() => reviewPauses.mutate({ project_id: projectId, review: true }, {
            onSuccess: ({ proposed }) => setRoughCutResult(proposed ? `${proposed} pauses ready for review` : "No new audio-confirmed pauses"),
            onError: (error) => setRoughCutResult(errText(error)),
          })}>
          {reviewPauses.isPending ? "Checking audio…" : "Review pauses"}
        </button>
        <button
          className="primary-btn"
          onClick={onRoughCut}
          disabled={!transcript || roughCut.isPending || reviewPauses.isPending || !!proposal}
          title={proposal ? "Apply or discard the current review first" : "Find pauses and filler words for review"}
        >
          {roughCut.isPending ? "Analyzing…" : "Review pauses and fillers"}
        </button>
        <span className="rough-cut-result" role="status" aria-atomic="true">
          {roughCut.isPending && roughStep ? `${roughStep}…` : roughCutResult}
        </span>
      </div>

      {proposal && proposal.cuts.length > 0 && (
        <CutReview
          projectId={projectId}
          proposal={proposal}
          protectedRanges={protectedRanges}
          frameRate={project?.media?.frame_rate}
          onDone={setRoughCutResult}
        />
      )}

      {(suggestions?.length ?? 0) > 0 && (
        <div className="suggest-review">
          <span className="suggest-review-label">
            {suggestions!.length} suggested cut{suggestions!.length === 1 ? "" : "s"} — repeated
            takes &amp; repetition, flagged for you (highlighted below)
          </span>
          <button
            className="primary-btn"
            disabled={suggestionReviewPending}
            onClick={() => acceptAll.mutate({ project_id: projectId }, suggestionReviewFeedback)}
          >
            {acceptAll.isPending ? "Preparing review…" : "Review all"}
          </button>
        </div>
      )}

      <div
        className="transcript-scroll"
        tabIndex={0}
        aria-label="Transcript"
        ref={parentRef}
        onContextMenu={(event) => {
          event.preventDefault();
          if (event.target instanceof HTMLElement) openContextMenu(event.target, event.clientX, event.clientY);
        }}
        onKeyDown={(event) => {
          const focusedElement = event.target;
          if (focusedElement instanceof HTMLElement && (focusedElement === event.currentTarget || focusedElement.matches("[data-segid]"))
              && (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) {
            event.preventDefault();
            const selection = window.getSelection();
            const bounds = selection?.rangeCount ? selection.getRangeAt(0).getBoundingClientRect() : undefined;
            const position = bounds ?? focusedElement.getBoundingClientRect();
            openContextMenu(focusedElement, position.left, position.bottom, true);
            return;
          }
          if (!event.shiftKey || event.altKey || event.ctrlKey || event.metaKey ||
              (event.key !== "ArrowLeft" && event.key !== "ArrowRight") ||
              event.target !== event.currentTarget) return;
          const words = Array.from(event.currentTarget.querySelectorAll<HTMLElement>("[data-wstart]"));
          if (!words.length) return;
          event.preventDefault();
          const existing = window.getSelection()?.isCollapsed === false;
          const anchorIndex = existing && wordAnchor.current ? words.indexOf(wordAnchor.current) : -1;
          const playhead = viewStore.state.playhead;
          const next = words.findIndex((word) => Number(word.dataset.wend) > playhead);
          const start = anchorIndex >= 0 ? anchorIndex : next >= 0 ? next : words.length - 1;
          const end = existing && keyboardWordEnd.current ? words.indexOf(keyboardWordEnd.current) : -1;
          const step = event.key === "ArrowLeft" ? -1 : 1;
          const target = end < 0 ? start : Math.max(0, Math.min(words.length - 1, end + step));
          wordAnchor.current = words[start]!;
          keyboardWordEnd.current = words[target]!;
          selectBetweenWords(wordAnchor.current, keyboardWordEnd.current);
          keyboardWordEnd.current.scrollIntoView({ block: "nearest" });
        }}
        onPointerDown={(event) => {
          if (event.button === 0 && (event.target as HTMLElement).closest("[data-wstart]")) {
            selectingText.current = true;
            if (event.shiftKey && wordAnchor.current?.isConnected) event.preventDefault();
          }
        }}
        onClickCapture={(event) => {
          const word = (event.target as HTMLElement).closest<HTMLElement>("[data-wstart]");
          if (!word) return;
          keyboardWordEnd.current = null;
          const anchor = wordAnchor.current;
          if (event.shiftKey && anchor?.isConnected && parentRef.current?.contains(anchor)) {
            selectBetweenWords(anchor, word);
            event.stopPropagation();
          } else if (window.getSelection()?.isCollapsed !== false) {
            wordAnchor.current = word;
          }
        }}
        onWheel={() => { userScrollAtRef.current = Date.now(); }}
        onTouchMove={() => { userScrollAtRef.current = Date.now(); }}
      >
        {visible.length === 0 ? (
            <div className="transcript-empty">
              {segments.length === 0 ? (
                <>
                  {transcriptLoadError ? (
                    <>
                      <p role="alert">{errText(transcriptLoadError)}</p>
                      <button className="ghost-btn" onClick={() => void refetchTranscript()}>Retry</button>
                    </>
                  ) : transcriptData === null && project?.media ? (
                    <TranscriptSourceActions key={project.media.id} project={project} media={project.media} />
                  ) : (
                    <p role="status">{transcriptData ? "No speech detected." : "Loading transcript…"}</p>
                  )}
                </>
              ) : (
                "No matching segments."
              )}
            </div>
        ) : (
          <div
            className="transcript-inner"
            style={{ height: virtualizer.getTotalSize(), position: "relative" }}
          >
            {virtualizer.getVirtualItems().map((item) => {
              const seg = visible[item.index]!;
              return (
                <div
                  key={seg.id}
                  data-index={item.index}
                  ref={virtualizer.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${item.start}px)`,
                  }}
                >
                  <TranscriptSegmentBlock
                    seg={seg}
                    cut={cutIds.has(seg.id)}
                    selected={selectedIds.includes(seg.id)}
                    active={activeSegId === seg.id}
                    fillerSet={fillerSet}
                    matchedWords={searching && !searchUpdating ? matches.get(seg.id)?.words : undefined}
                    onRestore={onRestore}
                    excludedRanges={excludedRanges}
                    protectedRanges={protectedRanges}
                    highlightedRanges={highlightedRanges}
                    showCuts={showCuts}
                    suggested={suggestionBySeg.get(seg.id)}
                    suggestionReviewPending={suggestionReviewPending}
                    onAcceptSuggestion={onAcceptSuggestion}
                    onDismissSuggestion={onDismissSuggestion}
                    silenceAfter={silenceAfter.get(seg.id)}
                    onRestoreRange={onRestoreRange}
                    clipDraft={clipDraft}
                    proposedCuts={proposedBySeg.get(seg.id) ?? NO_CUTS}
                    pausesBefore={pausesBySeg.get(seg.id)}
                    focusedCut={focusedCut}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>
      <ClipDraftBar projectId={projectId} />
      {segments.length > 0 && <div className="transcript-hint">
        {hasWordTimings
          ? "Click a word to seek · Shift-click or Shift+arrow to select"
          : "Click a timestamp to seek · Click a paragraph or focus it and press Enter to select"}
        {" · Right-click to cut or add to a clip · Delete cuts selection"}
      </div>}

      {selectionError && <p role="alert" className="empty-error">{selectionError}</p>}
      {correction && (
        <TranscriptCorrection
          key={JSON.stringify([correction.id, correction.text, correction.speaker])}
          projectId={projectId}
          segment={correction}
          close={() => setCorrection((current) => current === correction ? null : current)}
        />
      )}
      {menu && (
        <div
          className="context-menu"
          ref={contextMenuRef}
          style={{ left: menu.x, top: menu.y, maxHeight: `calc(100dvh - ${menu.y}px)` }}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.stopPropagation();
            setMenu(null);
            const origin = contextMenuOrigin.current;
            (origin?.isConnected ? origin : parentRef.current)?.focus({ preventScroll: true });
          }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          <div className="context-menu-label">{menu.label}</div>
          {menu.text && (
            <button onClick={() => {
              if (!navigator.clipboard) {
                setSelectionError("Clipboard access is unavailable. Use the system Copy shortcut.");
                return;
              }
              void navigator.clipboard.writeText(menu.text).then(
                () => {
                  setMenu(null);
                  if (menu.keyboard) {
                    const origin = contextMenuOrigin.current;
                    (origin?.isConnected ? origin : parentRef.current)?.focus({ preventScroll: true });
                  }
                },
                (error) => setSelectionError(errText(error)),
              );
            }}>
              Copy text
            </button>
          )}
          <button disabled={protectSourceRanges.isPending || menuPieces().length === 0} onClick={() => {
            protectSourceRanges.mutate({ project_id: projectId, transcript_id: menu.transcriptId, ranges: menuPieces() }, {
              onSuccess: finishMenuAction,
              onError: (error) => setSelectionError(errText(error)),
            });
          }}>
            Protect from automatic cuts
          </button>
          {menuRanges.some((piece) => overlapsRange(piece, protectedRanges)) && (
            <button disabled={protectSourceRanges.isPending} onClick={() => {
              protectSourceRanges.mutate({ project_id: projectId, transcript_id: menu.transcriptId, ranges: menuPieces(), remove: true }, {
                onSuccess: finishMenuAction,
                onError: (error) => setSelectionError(errText(error)),
              });
            }}>Remove protection from selection</button>
          )}
          {correctionTarget && (
            <button onClick={() => {
              setCorrection(correctionTarget);
              finishMenuAction();
            }}>
              Correct text
            </button>
          )}
          <button disabled={highlightSourceRanges.isPending || menuRanges.length === 0} onClick={() => {
            highlightSourceRanges.mutate({ project_id: projectId, transcript_id: menu.transcriptId, ranges: menuRanges, remove: menuIsHighlighted }, {
              onSuccess: finishMenuAction,
              onError: (error) => setSelectionError(errText(error)),
            });
          }}>
            {menuIsHighlighted ? "Remove highlight" : "Highlight only"}
          </button>
          <button onClick={menuCut} disabled={searchUpdating || cutRange.isPending || cutByTranscript.isPending}>
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
              <circle cx="4" cy="4" r="2.2" />
              <circle cx="4" cy="12" r="2.2" />
              <path d="M5.8 5.4 14 13M5.8 10.6 14 3" />
            </svg>
            Cut from video
          </button>
          {showCuts && <button onClick={menuRestore} disabled={restoreRange.isPending || restoreByTranscript.isPending}>
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
              <path d="M2.5 8a5.5 5.5 0 1 1 1.6 3.9" />
              <path d="M2.5 12V8h4" />
            </svg>
            Restore
          </button>}
          <button onClick={menuAddToClip}>
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
              <rect x="2.5" y="3.5" width="11" height="9" />
              <path d="M8 6v4M6 8h4" />
            </svg>
            Add to clip
          </button>
          {menuInClip && (
            <button onClick={menuRemoveFromClip}>
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
                <rect x="2.5" y="3.5" width="11" height="9" />
                <path d="M6 8h4" />
              </svg>
              Remove from clip
            </button>
          )}
          <button onClick={menuPlayHere}>
            <svg width="13" height="13" viewBox="0 0 16 16" fill="currentColor" stroke="none">
              <path d="M5 3.5v9l7.5-4.5z" />
            </svg>
            Play from here
          </button>
        </div>
      )}
    </div>
  );
}
