import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { getResolveCutPlan, openResolveReview } from "../ipc/api";
import { queryKeys } from "../ipc/queries";
import type { ResolveTimelineBinding } from "../ipc/types";
import { errText } from "./clipUtil";

export function OpenResolveReview({ onOpen }: { onOpen?: (id: string) => void }) {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const choose = async () => {
    setPending(true);
    setError("");
    try {
      const project = await openResolveReview();
      if (!project) return;
      queryClient.setQueryData(queryKeys.project(project.id), project);
      await queryClient.invalidateQueries({ queryKey: queryKeys.projects });
      onOpen?.(project.id);
    } catch (error) {
      setError(errText(error));
    } finally {
      setPending(false);
    }
  };
  return <>
    <button type="button" className="ghost-btn" disabled={pending} onClick={() => void choose()}>
      {pending ? "Opening Resolve review…" : "Open Resolve review"}
    </button>
    {error && <span role="alert">{error}</span>}
  </>;
}

export function ResolveReviewPlan({ projectId, binding }: {
  projectId: string;
  binding: ResolveTimelineBinding;
}) {
  const [pending, setPending] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const { timeline } = binding;
  const copy = async () => {
    setPending(true);
    setStatus("");
    setError("");
    try {
      const plan = await getResolveCutPlan(projectId);
      await navigator.clipboard.writeText(JSON.stringify(plan, null, 2));
      setStatus("Cut plan copied");
    } catch (error) {
      setError(errText(error));
    } finally {
      setPending(false);
    }
  };
  return <>
    <span title={timeline.timeline_name}>{timeline.start_timecode} · {(timeline.rate_num / timeline.rate_den).toFixed(3).replace(/\.?0+$/, "")} fps</span>
    <button type="button" className="ghost-btn" disabled={pending} onClick={() => void copy()}>
      Copy Resolve cut plan
    </button>
    {status && <span role="status">{status}</span>}
    {error && <span role="alert">{error}</span>}
  </>;
}
