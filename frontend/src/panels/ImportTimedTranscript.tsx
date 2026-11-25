import { useState } from "react";
import { useImportTimedTranscript } from "../ipc/queries";
import { errText } from "./clipUtil";

interface DialogueTrack {
  index: number;
  language?: string | null;
  channels?: number | null;
}

const MAX_TRANSCRIPT_BYTES = 16 * 1024 * 1024;

export function ImportTimedTranscript({ projectId, sourcePath, mediaId, audioStreams, boundDialogue }: {
  projectId: string;
  sourcePath: string;
  mediaId: string;
  audioStreams?: readonly DialogueTrack[];
  boundDialogue?: number;
}) {
  const [stream, setStream] = useState("");
  const mutation = useImportTimedTranscript();
  const [reading, setReading] = useState(false);
  const [error, setError] = useState("");
  const [confirmSourceClock, setConfirmSourceClock] = useState(false);
  const [file, setFile] = useState<File | null>(null);
  const isSrt = file?.name.toLowerCase().endsWith(".srt") ?? false;

  const importFile = async (file: File) => {
    setReading(true);
    setError("");
    try {
      const bytes = await file.arrayBuffer();
      let text: string;
      try {
        text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: boundDialogue !== undefined }).decode(bytes);
      } catch {
        throw new Error("Save the transcript as UTF-8, then choose it again.");
      }
      await mutation.mutateAsync({ project_id: projectId, text, format: file.name.toLowerCase().endsWith(".srt") ? "srt" : "json", confirm_source_clock: confirmSourceClock,
        confirmed_source_path: confirmSourceClock ? sourcePath : undefined,
        confirmed_media_id: confirmSourceClock ? mediaId : undefined,
        audio_stream_index: confirmSourceClock && stream !== "" ? Number(stream) : undefined });
    } catch (error) {
      setError(errText(error));
    } finally {
      setReading(false);
    }
  };

  return <div aria-busy={reading}>
    <label>Timed transcript <input type="file" accept=".json,.srt,application/json,application/x-subrip" disabled={reading}
      onChange={(event) => {
        const input = event.currentTarget;
        const selected = input.files?.[0] ?? null;
        const invalid = selected && (!/\.(json|srt)$/i.test(selected.name)
          ? "Choose a JSON or SRT transcript."
          : selected.size > MAX_TRANSCRIPT_BYTES ? "Choose a transcript file no larger than 16 MiB." : "");
        if (invalid) input.value = "";
        setFile(invalid ? null : selected);
        setConfirmSourceClock(false);
        setStream("");
        setError(invalid || "");
      }} /></label>
    {file && <>
      <label><input type="checkbox" checked={confirmSourceClock} disabled={reading}
        onChange={(event) => setConfirmSourceClock(event.target.checked)} />
        I confirm the timestamps match {sourcePath.split(/[\\/]/).pop() || "the attached source"}
      </label>
      {confirmSourceClock && audioStreams && audioStreams.length > 1 && <label>Dialogue track <select value={boundDialogue ?? stream} disabled={reading || boundDialogue !== undefined} onChange={(event) => setStream(event.target.value)}>
        <option value="">Choose a track</option>
        {audioStreams.map((audio, index) => (
          <option key={audio.index} value={audio.index}>
            Track {index + 1}{audio.language && ` · ${audio.language}`}
            {audio.channels && ` · ${audio.channels} ${audio.channels === 1 ? "channel" : "channels"}`}
          </option>
        ))}
      </select></label>}
      <button type="button" className="ghost-btn" disabled={reading || (isSrt && !confirmSourceClock) || (confirmSourceClock && boundDialogue === undefined && (audioStreams?.length ?? 0) > 1 && stream === "")} onClick={() => void importFile(file)}>
        {reading ? "Importing…" : "Import transcript"}
      </button>
    </>}
    {error && <p role="alert">{error}</p>}
  </div>;
}
