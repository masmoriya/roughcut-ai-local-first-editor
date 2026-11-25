// First-run media import. See ingest.ts.

import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { getSetupStatus, isTauri, type MediaSetupStatus } from "./ipc/api";
import { ingestFile } from "./ingest";
import { useProjects, useRestoreProject } from "./ipc/queries";
import { errText } from "./panels/clipUtil";
import { OpenResolveReview } from "./panels/ResolveReview";

/** The empty state is also where you land after trashing your LAST project —
 *  the trash must be reachable from here, not just the project switcher. */
function TrashList() {
  const projects = useProjects();
  const restore = useRestoreProject();
  const trash = projects.data?.trash ?? [];
  if (trash.length === 0) return null;
  return (
    <div className="empty-trash">
      <span className="empty-trash-label">Trash · kept for 30 days</span>
      {trash.map((p) => (
        <button
          key={p.id}
          className="empty-trash-item"
          disabled={restore.isPending}
          onClick={() => restore.mutate({ project_id: p.id })}
          title="Restore this project"
        >
          ↩ {p.name}
        </button>
      ))}
    </div>
  );
}

type Phase = "idle" | "working";

function SetupCard({ onChanged }: { onChanged: () => void }) {
  return (
    <div className="setup-card">
      <h3>FFmpeg required</h3>
      <p className="setup-note">Install FFmpeg to import video, then check again.</p>
      <button className="link-btn" onClick={onChanged}>Check again</button>
    </div>
  );
}

function baseName(path: string): string {
  const last = path.split(/[\\/]/).pop() ?? path;
  return last.replace(/\.[^.]+$/, "") || "Untitled project";
}

export function EmptyState() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [message, setMessage] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const [setup, setSetup] = useState<MediaSetupStatus | null>(null);
  const workingRef = useRef(false);
  const queryClient = useQueryClient();

  const refreshSetup = useCallback(() => {
    void getSetupStatus().then(setSetup).catch(() => setSetup(null));
  }, []);
  useEffect(refreshSetup, [refreshSetup]);

  const setupIncomplete = isTauri && !!setup && !setup.ffmpeg;

  const start = async (name: string, filePath: string) => {
    if (workingRef.current) return;
    workingRef.current = true;
    setError(null);
    setPhase("working");
    setMessage("Creating project…");
    try {
      await ingestFile(name, filePath, { onPhase: setMessage });
      await queryClient.invalidateQueries();
    } catch (err) {
      setError(errText(err));
      setPhase("idle");
    } finally {
      workingRef.current = false;
    }
  };

  // Tauri native file drop carries real paths.
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<{ paths: string[] }>("tauri://drag-drop", (e) => {
      const path = e.payload.paths?.[0];
      if (path) void start(baseName(path), path);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const choose = async () => {
    setError(null);
    try {
      const path = await openFileDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "avi", "m4v", "webm"] }],
      });
      if (typeof path === "string") await start(baseName(path), path);
    } catch (error) {
      setError(errText(error));
    }
  };

  return (
    <div className="empty-state">
      {phase === "idle" ? (
        <>
          {setupIncomplete && setup && <SetupCard onChanged={refreshSetup} />}
          <div
            className={`drop-zone${dragOver ? " over" : ""}`}
            onClick={() => void choose()}
            onDragOver={(e) => {
              e.preventDefault();
              setDragOver(true);
            }}
            onDragLeave={() => setDragOver(false)}
            onDrop={(event) => { event.preventDefault(); setDragOver(false); }}
          >
            <svg width="42" height="42" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.4">
              <rect x="2" y="4" width="20" height="16" rx="2.5" />
              <path d="M10.5 9.5v5l4-2.5z" fill="currentColor" stroke="none" />
            </svg>
            <h2>Drop a video or click to choose</h2>
            <p>Your footage never leaves this machine.</p>
          </div>
          {isTauri && <OpenResolveReview />}
          {error && <p className="empty-error" role="alert">{error}</p>}
          <TrashList />
        </>
      ) : (
        <div className="import-progress">
          <h2>Preparing your project</h2>
          <p className="progress-message">{message}</p>
        </div>
      )}
    </div>
  );
}
