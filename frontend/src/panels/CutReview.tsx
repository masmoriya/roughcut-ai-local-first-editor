// The review bar for proposed cuts: what the Rough cut button or the chat
// worked out, before anything is cut. Step through the cuts (each is marked
// in the transcript and timeline), hear one (a few seconds either side, with
// it taken out), leave cuts or whole kinds out, then Apply as one undoable
// edit — or Discard. The player already skips proposed cuts, so pressing
// play previews the edit as it would be.

import { useEffect, useMemo, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import {
  queryKeys,
  useApplyCutProposal,
  useDiscardCutProposal,
  useProtectSourceRanges,
  useSkipProposedCuts,
  useTimeline,
  useTranscript,
} from "../ipc/queries";
import type { CutProposal, ProposedCut } from "../ipc/types";
import { previewProposal, previewRanges, seekTo, setFocusedCut, setPlaying, setSkipCuts, viewStore } from "../state/viewStore";
import { excludedTotal, mergeRanges, overlapsRange } from "../playback/time";
import { errText } from "./clipUtil";
import { cutContext } from "./cutContext";
import { hasLowConfidence } from "./transcriptSearch";
import { RULE_LABELS } from "./ruleLabels";

/** Seconds of context played either side of a cut. */
const CONTEXT_S = 2.5;

const KINDS = [
  { kind: "pause", label: "pauses" },
  { kind: "filler", label: "filler words" },
  { kind: "words", label: "other words" },
  { kind: "fillers", label: "filler matches" },
  { kind: "profanity", label: "profanity matches" },
  { kind: "keywords", label: "keyword matches" },
  { kind: "search", label: "search matches" },
] as const;
function ruleLabel(cut: ProposedCut): string | undefined {
  return cut.rule_category ? RULE_LABELS[cut.rule_category] : undefined;
}

export function clock(s: number): string {
  const t = Math.max(0, s);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = Math.floor(t % 60);
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`
    : `${m}:${String(sec).padStart(2, "0")}`;
}

function span(s: number): string {
  return s >= 60 ? clock(s) : `${s.toFixed(1)}s`;
}

function what(c: ProposedCut): string {
  if (c.kind === "pause") return `pause · ${span(c.end - c.start)}`;
  return `“${c.text}” · ${span(c.end - c.start)}`;
}

/** Hear a cut as it would be: a few seconds before, then straight on after. */
export function hearCut(c: ProposedCut, keepPassage = c.skipped): void {
  setSkipCuts(true);
  previewRanges(
    keepPassage
      ? [{ start: Math.max(0, c.start - CONTEXT_S), end: c.end + CONTEXT_S }]
      : [
          { start: Math.max(0, c.start - CONTEXT_S), end: c.start },
          { start: c.end, end: c.end + CONTEXT_S },
        ],
  );
}

export function CutReview({
  projectId,
  proposal,
  protectedRanges,
  frameRate,
  onDone,
}: {
  projectId: string;
  proposal: CutProposal;
  protectedRanges: readonly { start: number; end: number }[];
  frameRate?: number;
  /** A one-line result for the toolbar after Apply or Discard. */
  onDone: (message: string) => void;
}) {
  const { data: timeline } = useTimeline(projectId);
  const { data: transcript } = useTranscript(projectId);
  const apply = useApplyCutProposal();
  const queryClient = useQueryClient();
  const discard = useDiscardCutProposal();
  const skip = useSkipProposedCuts();
  const protect = useProtectSourceRanges();
  const focusedId = useStore(viewStore, (s) => s.focusedCut);
  const [showList, setShowList] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [matchSelection, setMatchSelection] = useState<{ cutId: number; index: number }>();

  const cuts = proposal.cuts;
  const protectedIds = useMemo(() => new Set(cuts.filter((cut) =>
    overlapsRange(cut, protectedRanges),
  ).map((cut) => cut.id)), [cuts, protectedRanges]);
  const eligible = useMemo(() => cuts.filter((cut) => !protectedIds.has(cut.id)), [cuts, protectedIds]);
  const live = useMemo(() => eligible.filter((c) => !c.skipped), [eligible]);
  const proposed = useMemo(() => mergeRanges(live), [live]);
  const included = useMemo(() => mergeRanges((timeline?.clips ?? [])
    .filter((clip) => clip.included)
    .map((clip) => ({ start: clip.source_in, end: clip.source_out }))), [timeline]);
  const refreshingProject = useIsFetching({ queryKey: queryKeys.project(projectId), exact: true }) > 0;
  const busy = refreshingProject || apply.isPending || discard.isPending || skip.isPending || protect.isPending;
  const wordMidpoints = useMemo(
    () => transcript?.segments.flatMap((segment) => segment.words.map((word) => (word.start + word.end) / 2))
      .filter(Number.isFinite).sort((a, b) => a - b) ?? [],
    [transcript],
  );
  const wordsRemoved = useMemo(() => {
    let nextCut = 0;
    let nextIncluded = 0;
    let count = 0;
    for (const time of wordMidpoints) {
      while (proposed[nextCut] && proposed[nextCut]!.end <= time) nextCut++;
      const cut = proposed[nextCut];
      if (!cut) break;
      if (time < cut.start) continue;
      while (included[nextIncluded] && included[nextIncluded]!.end <= time) nextIncluded++;
      const kept = included[nextIncluded];
      if (!timeline || (kept && time >= kept.start)) count++;
    }
    return count;
  }, [timeline, proposed, included, wordMidpoints]);
  const removes = useMemo(() => {
    if (!timeline) return excludedTotal(proposed);
    let duration = 0;
    let clipIndex = 0;
    let cutIndex = 0;
    while (clipIndex < included.length && cutIndex < proposed.length) {
      const clip = included[clipIndex]!;
      const cut = proposed[cutIndex]!;
      duration += Math.max(0, Math.min(clip.end, cut.end) - Math.max(clip.start, cut.start));
      if (clip.end < cut.end) clipIndex++;
      else cutIndex++;
    }
    return duration;
  }, [timeline, proposed, included]);
  const before = excludedTotal(included);
  const previewDuration = Math.max(0, before - removes);
  const framesRemoved = frameRate && Number.isFinite(frameRate) && frameRate > 0
    ? Math.round(removes * frameRate)
    : 0;
  const canPlayEdit = timeline !== undefined && previewDuration > Number.EPSILON;
  const idx = Math.max(0, cuts.findIndex((c) => c.id === focusedId));
  const focused = cuts[idx];
  const context = useMemo(() => focused && transcript
    ? cutContext(transcript.segments, focused.start, focused.end)
    : undefined, [focused, transcript]);
  const isProtected = (cut: ProposedCut) => protectedIds.has(cut.id);
  const matchIndex = matchSelection?.cutId === focused?.id ? (matchSelection?.index ?? 1) : 1;
  const selectedMatch = focused?.phrase_matches?.[matchIndex - 1];
  const pageSize = 50;
  const pageStart = Math.floor(idx / pageSize) * pageSize;
  const pageEnd = Math.min(pageStart + pageSize, cuts.length);

  // Land on the first cut when a proposal arrives; clear on the way out.
  useEffect(() => {
    if (!cuts.some((c) => c.id === focusedId)) setFocusedCut(cuts[0]?.id ?? null);
  }, [cuts, focusedId]);
  useEffect(() => () => setFocusedCut(null), []);

  const go = (i: number) => {
    const c = cuts[(i + cuts.length) % cuts.length];
    if (!c) return;
    setFocusedCut(c.id);
    setPlaying(false);
    seekTo(Math.max(0, c.start - 0.5));
  };

  const run = <T,>(p: Promise<T>) => {
    setError(null);
    return p.catch((e) => {
      setError(errText(e));
      void queryClient.invalidateQueries({ queryKey: queryKeys.project(projectId) });
    });
  };

  const toggleKind = (kind: (typeof KINDS)[number]["kind"]) => {
    const ofKind = eligible.filter((c) => (c.rule_category ?? c.kind) === kind);
    const allOn = ofKind.every((c) => !c.skipped);
    void run(skip.mutateAsync({ project_id: projectId, proposal_id: proposal.id, cut_ids: ofKind.map((cut) => cut.id), skip: allOn }));
  };
  const toggleCut = (c: ProposedCut) =>
    void run(skip.mutateAsync({ project_id: projectId, proposal_id: proposal.id, cut_ids: [c.id], skip: !c.skipped }));

  const onApply = () => {
    setError(null);
    apply.mutate(
      { project_id: projectId, proposal_id: proposal.id, selected_cut_ids: live.map((cut) => cut.id) },
      {
        onSuccess: (r) =>
          onDone(
            r.applied > 0
              ? `applied ${r.applied} cut${r.applied === 1 ? "" : "s"} (${span(r.seconds)}) · undo with ⌘Z`
              : "every cut was skipped — nothing cut",
          ),
        onError: (e) => {
          setError(errText(e));
          void queryClient.invalidateQueries({ queryKey: queryKeys.project(projectId) });
        },
      },
    );
  };
  const onDiscard = () => {
    setError(null);
    discard.mutate(
      { project_id: projectId, proposal_id: proposal.id },
      { onSuccess: () => onDone("Proposal discarded"), onError: (e) => {
        setError(errText(e));
        void queryClient.invalidateQueries({ queryKey: queryKeys.project(projectId) });
      } },
    );
  };

  const playEdit = () => {
    previewProposal();
    seekTo(Math.max(0, (focused?.start ?? 0) - CONTEXT_S));
    setPlaying(true);
  };

  return (
    <div className="cut-review" role="region" aria-label="Proposed cuts">
      <div className="cut-review-head">
        <span className="cut-review-tag">proposed</span>
        <span className="cut-review-title">{proposal.title}</span>
        <span className="cut-review-sum" aria-live="polite" aria-atomic="true">
          {live.length} cut{live.length === 1 ? "" : "s"} · removes {span(removes)}
          {framesRemoved > 0 && ` · ≈${framesRemoved} ${framesRemoved === 1 ? "frame" : "frames"}`}
          {transcript && ` · ${wordsRemoved} ${wordsRemoved === 1 ? "word" : "words"}`}
          {before > 0 && ` · ${clock(before)} → ${clock(previewDuration)}`}
        </span>
        <span className="cut-review-spacer" />
        <button className="ghost-btn" onClick={onDiscard} disabled={busy}>
          Discard
        </button>
        <button className="primary-btn" onClick={onApply} disabled={busy || live.length === 0 || removes <= Number.EPSILON}>
          {apply.isPending ? "Applying…" : live.length === 0 ? "No cuts selected" : removes <= Number.EPSILON
            ? "No footage to remove"
            : `Apply ${live.length} cut${live.length === 1 ? "" : "s"}`}
        </button>
      </div>

      <div className="cut-review-kinds">
        {KINDS.map(({ kind, label }) => {
          const ofKind = eligible.filter((c) => (c.rule_category ?? c.kind) === kind);
          if (ofKind.length === 0) return null;
          const on = ofKind.filter((c) => !c.skipped);
          const secs = on.reduce((a, c) => a + c.end - c.start, 0);
          const state = on.length === ofKind.length ? "on" : on.length === 0 ? "off" : "some";
          return (
            <button
              key={kind}
              className={`cut-kind ${state}`}
              onClick={() => toggleKind(kind)}
              disabled={busy}
              aria-pressed={state === "some" ? "mixed" : state === "on"}
              title={state === "on" ? `Keep all ${label}` : `Select all ${label} for removal`}
            >
              <span className="cut-kind-box">{state === "on" ? "■" : state === "some" ? "▪" : " "}</span>
              {label} {on.length}
              {on.length !== ofKind.length && `/${ofKind.length}`} · {span(secs)}
            </button>
          );
        })}
        {eligible.length > 1 && (
          <button
            className="ghost-btn"
            disabled={busy}
            onClick={() => void run(skip.mutateAsync({ project_id: projectId, proposal_id: proposal.id, cut_ids: eligible.map((cut) => cut.id), skip: live.length > 0 }))}
          >
            {live.length > 0 ? "Keep all" : "Select all cuts"}
          </button>
        )}
        <span className="cut-review-spacer" />
        <button className="ghost-btn" onClick={playEdit} disabled={busy || !canPlayEdit}
          title={canPlayEdit ? "Play from this cut with the selected cuts removed" : "No retained footage to preview"}>
          ▶ play the edit
        </button>
        <button className="ghost-btn" onClick={() => setShowList((v) => !v)}>
          {showList ? "Hide list" : "Show list"}
        </button>
      </div>

      {focused && (
        <div className={`cut-review-nav${focused.skipped ? " skipped" : ""}`}>
          <button className="ghost-btn" onClick={() => go(idx - 1)} aria-label="Previous cut">
            ‹
          </button>
          <span className="cut-review-pos">
            {idx + 1}/{cuts.length}
          </span>
          <button className="cut-review-at" onClick={() => go(idx)} title="Show it in the transcript and timeline">
            {clock(focused.start)}
          </button>
          <span className="cut-review-what">
            {ruleLabel(focused) ? `${ruleLabel(focused)} · ` : ""}
            {what(focused)}
            {focused.phrase_matches && focused.phrase_matches.length > 1 &&
              ` · ${focused.phrase_matches.length} overlapping matches`}
            {hasLowConfidence(focused.min_word_confidence) && " · low confidence"}
            {isProtected(focused) ? " · protected" : focused.skipped && " · kept"}
          </span>
          <button className="ghost-btn" disabled={busy}
            onClick={() => hearCut(focused, focused.skipped || isProtected(focused))}
            title={focused.skipped || isProtected(focused) ? "Play the retained passage with context" : "Preview the cut with context"}>
            ▶ hear it
          </button>
          <button className="ghost-btn" onClick={() => toggleCut(focused)} disabled={busy || isProtected(focused)}>
            {focused.skipped ? "cut it" : "keep it"}
          </button>
          <button className="ghost-btn" disabled={busy} onClick={() => void run(protect.mutateAsync({
            project_id: projectId,
            proposal_id: proposal.id,
            transcript_id: proposal.transcript_id ?? undefined,
            ranges: [{ start: focused.start, end: focused.end }],
          }))} title="Keep this range out of future automatic cut reviews">
            Protect
          </button>
          <button className="ghost-btn" onClick={() => go(idx + 1)} aria-label="Next cut">
            ›
          </button>
        </div>
      )}
      {context && (context.before || context.after) && (
        <p className="cut-review-context" aria-label="Transcript around the focused cut">
          {context.before} … {context.after}
        </p>
      )}
      {focused?.phrase_matches && focused.phrase_matches.length > 1 && !focused.skipped && !isProtected(focused) && frameRate && Number.isFinite(frameRate) && frameRate > 0 && (
        <details className="cut-review-context" key={focused.id}>
          <summary>Protect a phrase match</summary>
          <form onSubmit={(event) => {
            event.preventDefault();
            if (busy) return;
            if (!selectedMatch || selectedMatch.source_out_frame <= selectedMatch.source_in_frame) return;
            void run(protect.mutateAsync({ project_id: projectId, proposal_id: proposal.id, split_cut_id: focused.id, transcript_id: proposal.transcript_id ?? undefined, ranges: [{
              start: selectedMatch.source_in_frame / frameRate, end: selectedMatch.source_out_frame / frameRate,
            }] }).then(() => onDone("Phrase protected")));
          }}>
            <label>Match <input type="number" min={1} max={focused.phrase_matches.length}
              value={matchIndex || ""} required disabled={busy}
              onChange={(event) => setMatchSelection({ cutId: focused.id, index: Number(event.currentTarget.value) })} /></label>
            {selectedMatch && <button type="button" className="ghost-btn" onClick={() => {
              setPlaying(false);
              seekTo(selectedMatch.source_in_frame / frameRate);
            }} aria-label="Show the selected phrase match in the source">
              {clock(selectedMatch.source_in_frame / frameRate)}–{clock(selectedMatch.source_out_frame / frameRate)}
            </button>}
            <button type="submit" className="ghost-btn" disabled={busy || !selectedMatch}>Protect match</button>
          </form>
          <p>Protecting a match updates the review with the remaining unprotected portions.</p>
        </details>
      )}

      {showList && (
        <>
        {cuts.length > pageSize && (
          <div className="cut-review-nav" aria-label="Cut list pages">
            <button className="ghost-btn" disabled={pageStart === 0} onClick={() => go(pageStart - pageSize)}>Previous page</button>
            <span role="status">{pageStart + 1}–{pageEnd} of {cuts.length}</span>
            <button className="ghost-btn" disabled={pageEnd === cuts.length} onClick={() => go(pageEnd)}>Next page</button>
          </div>
        )}
        <ol className="cut-review-list" start={pageStart + 1}>
          {cuts.slice(pageStart, pageEnd).map((c, index) => (
            <li key={c.id} className={`${c.id === focusedId ? "focused" : ""}${c.skipped ? " skipped" : ""}`}>
              <button className="cut-review-at" onClick={() => go(pageStart + index)}
                aria-current={c.id === focusedId ? "true" : undefined}>
                {clock(c.start)}
              </button>
              <span className={`cut-review-kind k-${c.kind}`}>{ruleLabel(c) ?? c.kind}</span>
              <span className="cut-review-what">{what(c)}{isProtected(c) && " · protected"}</span>
              <button className="ghost-btn" disabled={busy}
                onClick={() => hearCut(c, c.skipped || isProtected(c))} aria-label={`Listen at ${clock(c.start)}`}>
                ▶
              </button>
              <button className="ghost-btn" onClick={() => toggleCut(c)} disabled={busy || isProtected(c)}
                aria-label={`${c.skipped ? "Cut" : "Keep"} passage at ${clock(c.start)}. ${c.skipped ? "Currently kept." : "Selected for removal."}`}>
                {c.skipped ? "cut" : "keep"}
              </button>
            </li>
          ))}
        </ol>
        </>
      )}
      {error && <div className="cut-review-error" role="alert">{error}</div>}
    </div>
  );
}
