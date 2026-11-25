// Footage tab: extend the CURRENT project with another video file, appended
// to the end of the source. Existing cuts keep their timestamps.

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { callTool, isTauri } from "../ipc/api";
import { useProject } from "../ipc/queries";
import { errText } from "./clipUtil";

export function FootagePanel({ projectId }: { projectId: string }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const queryClient = useQueryClient();
  const { data: project } = useProject(projectId);
  const audioMetadata = project?.media as { source_audio_tracks?: number; audio_streams?: { index: number; channels: number | null; language: string | null }[] } | undefined;
  const audioStreams = audioMetadata?.audio_streams;
  const sourcePath = project?.media?.file_path;

  const refreshSource = async (path: string) => {
    setBusy(path);
    setError(null);
    setStatus("Relinking source…");
    try {
      await callTool("import_media", { project_id: projectId, file_path: path });
      await queryClient.invalidateQueries({ predicate: (query) => query.queryKey[1] === projectId });
      setStatus("Source linked");
    } catch (error) {
      setStatus("");
      setError(errText(error));
    } finally {
      setBusy(null);
    }
  };

  const append = async (label: string, path: string) => {
    setBusy(path);
    setError(null);
    setStatus("Appending video…");
    try {
      await callTool("append_media", { project_id: projectId, file_path: path });
      await queryClient.invalidateQueries({ predicate: (query) => query.queryKey[1] === projectId });
      setStatus("Video appended");
    } catch (error) {
      setStatus("");
      setError(`${label}: ${errText(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const browse = async (mode: "append" | "relink") => {
    if (busy) return;
    setBusy(mode);
    setError(null);
    setStatus("");
    try {
      const path = await openFileDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "Video", extensions: ["mp4", "mov", "m4v", "mkv", "avi", "webm"] }],
      });
      if (typeof path !== "string") return;
      if (mode === "append") {
        await append(path.split(/[\\/]/).pop() || path, path);
        return;
      }
      await refreshSource(path);
    } catch (error) {
      setError(errText(error));
    } finally {
      setBusy(null);
    }
  };

  if (!isTauri) {
    return <p className="footage-note">Adding footage needs the desktop app.</p>;
  }

  return (
    <div className="footage-panel">
      {!!audioStreams?.length && <details>
        <summary>Source audio ({audioStreams.length})</summary>
        <ul>{audioStreams.map((stream) => <li key={stream.index}>
          Stream {stream.index}{stream.channels != null && ` · ${stream.channels} channels`}{stream.language && ` · ${stream.language}`}
        </li>)}</ul>
      </details>}
      {sourcePath && !audioStreams?.length && audioMetadata?.source_audio_tracks !== 0 && (
        <button className="ghost-btn" disabled={!!busy} onClick={() => void refreshSource(sourcePath)}>
          Refresh source audio
        </button>
      )}
      <p className="footage-note">
        Append footage with the same codec and resolution. Existing cuts keep their timing.
      </p>
      <div className="footage-actions">
        <button className="ghost-btn" onClick={() => void browse("append")} disabled={!!busy}>
          Add video
        </button>
        <button className="ghost-btn" onClick={() => void browse("relink")} disabled={!!busy}>
          Relink source
        </button>
      </div>
      {error && <p className="empty-error" role="alert">{error}</p>}
      <span role="status">{status}</span>
    </div>
  );
}
