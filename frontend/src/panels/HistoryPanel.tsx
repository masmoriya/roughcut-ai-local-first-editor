// Edit history — "go back in time". A view over the journal (useHistory):
// every applied edit then every undone/future edit, newest at top. Click any
// row to jump there (jump_to undoes/redoes to land exactly on it). The
// current point is marked; future (undone) rows render dimmed.

import { useState } from "react";
import { useHistory, useHistoryCutReview, useJumpTo } from "../ipc/queries";
import { errText } from "./clipUtil";
import { RULE_LABELS } from "./ruleLabels";

const SOURCE_ICON: Record<string, string> = {
  ui: "✋",
  local_ai: "💬",
  mcp_client: "🔌",
};

function relativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const s = Math.max(0, (Date.now() - then) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function SavedCutReview({ projectId, actionId }: { projectId: string; actionId: string }) {
  const [page, setPage] = useState(0);
  const [expandedMatchCut, setExpandedMatchCut] = useState<number | null>(null);
  const pageSize = 50;
  const first = page * pageSize;
  const review = useHistoryCutReview(projectId, actionId, first);
  if (review.isPending) return <p role="status">Loading review…</p>;
  if (review.isError) return <p role="alert">{errText(review.error)}</p>;
  if (!review.data) return <p>No saved review.</p>;
  const { decisions, total } = review.data;
  return (
    <>
    <ul>
      {decisions.map((decision) => (
        <li key={decision.cut_id}>
          <strong>{decision.approved ? "Cut" : "Kept"}</strong>{" "}
          {decision.rule_category && `· ${RULE_LABELS[decision.rule_category] ?? decision.rule_category} `}
          · Frames {decision.source_in_frame}–{decision.source_out_frame}{" "}
          {typeof decision.min_word_confidence === "number" &&
            `· ${Math.round(decision.min_word_confidence * 100)}% minimum word confidence `}
          {[decision.context_before, decision.context_after].filter(Boolean).join(" … ")}
          {decision.phrase_matches && decision.phrase_matches.length > 1 && (
            <details open={expandedMatchCut === decision.cut_id}>
              <summary onClick={(event) => {
                event.preventDefault();
                setExpandedMatchCut(expandedMatchCut === decision.cut_id ? null : decision.cut_id);
              }}>{decision.phrase_matches.length} phrase matches</summary>
              {expandedMatchCut === decision.cut_id && <ul>
                {decision.phrase_matches.map((match) => (
                  <li key={match.match_id}>
                    Frames {match.source_in_frame}–{match.source_out_frame}{" "}
                    {[match.context_before, match.context_after].filter(Boolean).join(" … ")}
                  </li>
                ))}
              </ul>}
            </details>
          )}
        </li>
      ))}
    </ul>
    {total > pageSize && (
      <div>
        <button type="button" disabled={page === 0} onClick={() => setPage(page - 1)}>Previous</button>
        <span> {first + 1}–{first + decisions.length} of {total} </span>
        <button type="button" disabled={first + decisions.length >= total}
          onClick={() => setPage(page + 1)}>Next</button>
      </div>
    )}
    </>
  );
}

export function HistoryPanel({ projectId }: { projectId: string }) {
  const { data: history } = useHistory(projectId);
  const jump = useJumpTo();
  const [openReview, setOpenReview] = useState<string | null>(null);

  const entries = history?.entries ?? [];
  const current = history?.current_index ?? -1;
  const hasFuture = entries.some((e) => !e.applied);
  const jumpTo = (id: string | null) =>
    jump.mutate({ project_id: projectId, action_id: id });
  const jumpError = jump.isError
    ? errText(jump.error)
    : null;

  // Newest first for reading; remember each row's original chronological index
  // so we can compare against current_index.
  const rows = entries.map((e, i) => ({ e, i })).reverse();

  return (
    <div className="history-panel">
      <p className="history-note">
        Every edit, newest first. Click any point to jump back (or forward) in
        time.
        {hasFuture
          ? " Heads up: making a new edit from an earlier point discards the dimmed steps below it."
          : ""}
      </p>
      {jumpError && <p className="empty-error history-error">jump failed: {jumpError}</p>}
      <div className="history-list">
        {rows.map(({ e, i }) => {
          const isCurrent = i === current;
          const isFuture = !e.applied;
          return (
            <div key={e.id}>
            <button
              className={`history-row${isCurrent ? " current" : ""}${isFuture ? " future" : ""}`}
              disabled={jump.isPending || isCurrent}
              onClick={() => jumpTo(e.id)}
              title={isFuture ? "Jump forward to here (redo to this point)" : "Jump back to here"}
            >
              <span className="history-src" title={e.source}>
                {SOURCE_ICON[e.source] ?? "•"}
              </span>
              <span className="history-desc">{e.description}</span>
              <span className="history-time">{relativeTime(e.timestamp)}</span>
              {isCurrent && <span className="history-now">now</span>}
            </button>
            {e.has_cut_review && (
              <>
                <button type="button" aria-expanded={openReview === e.id}
                  onClick={() => setOpenReview(openReview === e.id ? null : e.id)}>
                  {openReview === e.id ? "Hide review" : "Show review"}
                </button>
                {openReview === e.id && <SavedCutReview projectId={projectId} actionId={e.id} />}
              </>
            )}
            </div>
          );
        })}
        <button
          className={`history-row original${current === -1 ? " current" : ""}`}
          disabled={jump.isPending || current === -1}
          onClick={() => jumpTo(null)}
          title="Jump all the way back to the original, before any edit"
        >
          <span className="history-src">↩</span>
          <span className="history-desc">Original — before any edit</span>
          {current === -1 && <span className="history-now">now</span>}
        </button>
      </div>
    </div>
  );
}
