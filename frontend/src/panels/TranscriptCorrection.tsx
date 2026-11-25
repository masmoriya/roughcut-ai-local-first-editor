import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { callTool } from "../ipc/api";
import { queryKeys } from "../ipc/queries";
import type { TranscriptSegment } from "../ipc/types";
import { errText } from "./clipUtil";

export function TranscriptCorrection({ projectId, segment, close }: {
  projectId: string;
  segment: TranscriptSegment;
  close: () => void;
}) {
  const queryClient = useQueryClient();
  const [text, setText] = useState(segment.words.length
    ? segment.words.map((word) => word.text).join(" ")
    : segment.text);
  const [speaker, setSpeaker] = useState(segment.speaker ?? "");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  return (
    <form onSubmit={async (event) => {
      event.preventDefault();
      setSaving(true);
      setError("");
      try {
        await callTool("correct_transcript_segment", {
          project_id: projectId,
          segment_id: segment.id,
          original: segment.text,
          original_speaker: segment.speaker ?? "",
          text,
          speaker,
        });
        await queryClient.invalidateQueries({ queryKey: queryKeys.transcript(projectId) });
        close();
      } catch (err) {
        setError(errText(err));
      } finally {
        setSaving(false);
      }
    }}>
      <label>
        Correct transcript text
        <textarea autoFocus value={text} onChange={(event) => setText(event.target.value)} disabled={saving} />
      </label>
      <label>Speaker <input value={speaker} maxLength={80} disabled={saving}
        onChange={(event) => setSpeaker(event.target.value)} /></label>
      <button type="submit" disabled={saving}>Save correction</button>
      <button type="button" disabled={saving} onClick={close}>Cancel</button>
      {error && <p role="alert">{error}</p>}
    </form>
  );
}
