import { useHighlightSourceRanges, useProtectSourceRanges, useReviewSearchCuts } from "../ipc/queries";
import type { TimedPhraseMatch } from "./transcriptSearch";
import { errText } from "./clipUtil";

export function TranscriptSearchActions({ projectId, transcriptId, ranges, category, visible, updating, onResult }: {
  projectId: string;
  transcriptId: string;
  ranges: TimedPhraseMatch[];
  category: "fillers" | "profanity" | "keywords" | "search";
  visible: boolean;
  updating: boolean;
  onResult: (message: string) => void;
}) {
  const review = useReviewSearchCuts();
  const highlight = useHighlightSourceRanges();
  const protect = useProtectSourceRanges();
  if (!visible) return null;
  const busy = updating || review.isPending || highlight.isPending || protect.isPending;
  const scope = { project_id: projectId, transcript_id: transcriptId };
  const onError = (error: unknown) => onResult(errText(error));

  const annotate = (kind: "highlight" | "protect") => {
    if (ranges.length > 1000) {
      onResult("Narrow the search to 1,000 timed matches.");
      return;
    }
    const args = { ...scope, ranges: ranges.map(({ start, end }) => ({ start, end })) };
    if (kind === "highlight") {
      highlight.mutate(args, { onSuccess: () => onResult("Highlights saved"), onError });
    } else {
      protect.mutate(args, { onSuccess: () => onResult("Protection saved"), onError });
    }
  };

  return (
    <>
      <button type="button" className="ghost-btn" disabled={busy} onClick={() =>
        review.mutate({ ...scope, ranges, category }, {
          onSuccess: ({ proposed }) => onResult(proposed ? `${proposed} cuts ready for review` : "No new cuts to review"),
          onError,
        })}>
        Review cuts
      </button>
      <button type="button" className="ghost-btn" disabled={busy} onClick={() => annotate("highlight")}>
        Highlight matches
      </button>
      <button type="button" className="ghost-btn" disabled={busy} onClick={() => annotate("protect")}>
        Protect matches
      </button>
    </>
  );
}
