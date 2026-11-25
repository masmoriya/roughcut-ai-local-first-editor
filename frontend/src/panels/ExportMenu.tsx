// Export button + dropdown of targets. Calls the `export` tool with a
// sensible Downloads path for the chosen format.

import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { isTauri, onAppEvent, revealPath } from "../ipc/api";
import type { ProgressEvent } from "../ipc/types";
import { useExport } from "../ipc/queries";
import type { ExportResult } from "../ipc/types";
import type { ExportTarget } from "../ipc/types";
import { errText } from "./clipUtil";

export const EXPORT_TARGETS: { target: ExportTarget; label: string; ext: string }[] = [
  { target: "premiere_xml", label: "Premiere XML", ext: "xml" },
  { target: "fcp_xml", label: "Final Cut XML", ext: "fcpxml" },
  { target: "resolve_xml", label: "DaVinci Resolve timeline", ext: "xml" },
  { target: "resolve_manifest", label: "Resolve cut manifest", ext: "json" },
  { target: "edl", label: "EDL", ext: "edl" },
  { target: "otio", label: "OTIO", ext: "otio" },
  { target: "mp4", label: "MP4", ext: "mp4" },
  { target: "srt", label: "SRT captions", ext: "srt" },
  { target: "txt", label: "Transcript (.txt)", ext: "txt" },
  { target: "transcript_json", label: "Timed transcript (.json)", ext: "json" },
];

export function ExportMenu({
  projectId,
  projectName,
  sourceAudioTracks,
  sourceAudioChannels,
  blocked,
}: {
  projectId: string;
  projectName: string;
  sourceAudioTracks?: number;
  sourceAudioChannels?: number;
  /** Why the whole video can't be exported right now (a clip is open). */
  blocked?: string;
}) {
  const [open, setOpen] = useState(false);
  const [choosingPath, setChoosingPath] = useState(false);
  const dialogOpen = useRef(false);
  const lastResolvePlan = useRef<{ projectId: string; version: string; frames: number }>();
  const [status, setStatus] = useState<{
    text: string;
    path?: string;
    revealError?: string;
    manifestStatus?: string;
    showManifest?: boolean;
    fraction?: number;
    busy?: boolean;
    retry?: (typeof EXPORT_TARGETS)[number];
    resolveTimeline?: ExportResult["resolve_timeline"];
    resolvePlanChange?: string;
  } | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const exportMutation = useExport();
  const resolveAudioLimit = (sourceAudioTracks ?? 0) > 1
    ? "requires one audio track"
    : (sourceAudioChannels ?? 0) > 2 ? "requires mono or stereo audio" : undefined;

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  // Live render progress from the core (ffmpeg percentage).
  useEffect(() => {
    return onAppEvent<ProgressEvent>("progress", (p) => {
      if (p.task !== "export" || p.project_id !== projectId) return;
      setStatus((cur) =>
        cur?.busy
          ? { ...cur, text: p.message, fraction: p.fraction }
          : cur,
      );
    });
  }, [projectId]);

  const slug = projectName.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "project";

  const run = async (target: ExportTarget, ext: string, label: string) => {
    if (!isTauri || blocked || dialogOpen.current || exportMutation.isPending) return;
    setOpen(false);
    let outPath: string;
    {
      dialogOpen.current = true;
      setChoosingPath(true);
      try {
        const selected = await save({
          defaultPath: `${slug}-${["srt", "txt", "transcript_json"].includes(target) ? "transcript" : "edited"}.${ext}`,
          filters: [{ name: label, extensions: [ext] }],
        });
        if (!selected) return;
        outPath = selected;
      } catch (err) {
        setStatus({ text: `Export failed: ${errText(err)}`, retry: { target, ext, label } });
        return;
      } finally {
        dialogOpen.current = false;
        setChoosingPath(false);
      }
    }
    setStatus({ text: `Exporting ${label}…`, busy: true, fraction: 0 });
    exportMutation.mutate(
      {
        project_id: projectId,
        target,
        out_path: outPath,
      },
      {
        onSuccess: (res) => {
          const filename = res.path.split(/[\\/]/).pop() || res.path;
          let resolvePlanChange: string | undefined;
          if (res.resolve_timeline) {
            const { cut_plan_version: version, timeline_frames: frames } = res.resolve_timeline;
            if (version) {
              const previous = lastResolvePlan.current;
              if (previous?.projectId === projectId) {
                resolvePlanChange = previous.version === version
                  ? "Same cut plan as the previous export."
                  : previous.frames === frames
                    ? "Cut plan changed. Timeline frame count stayed the same."
                    : `Cut plan changed from ${previous.frames} to ${frames} timeline frames.`;
              }
              lastResolvePlan.current = { projectId, version, frames };
            }
          }
          setStatus({
            text: target === "resolve_xml"
              ? `Saved ${filename} · Import as a new timeline in Resolve`
              : `Saved ${filename}`,
            path: res.path,
            busy: false,
            resolveTimeline: res.resolve_timeline,
            resolvePlanChange,
          });
        },
        onError: (err) =>
          setStatus({ text: `Export failed: ${errText(err)}`, retry: { target, ext, label } }),
      },
    );
  };


  return (
    <div className="export-menu" ref={rootRef}>
      {status && (
        <div className="export-status" title={status.path}>
          {status.busy && (
            <span className="export-bar">
              <span
                className="export-bar-fill"
                style={{ width: `${Math.round((status.fraction ?? 0) * 100)}%` }}
              />
            </span>
          )}
          <span role="status">{status.text}</span>
          {status.retry && (
            <button className="reveal-btn" disabled={!!blocked || choosingPath || exportMutation.isPending} onClick={() => {
              const { target, ext, label } = status.retry!;
              void run(target, ext, label);
            }}>
              Retry export
            </button>
          )}
          {status.path && (
            <button className="reveal-btn" onClick={async () => {
              const path = status.path;
              if (!path) return;
              try {
                await revealPath(path);
                setStatus((current) => current?.path === path ? { ...current, revealError: undefined } : current);
              } catch {
                setStatus((current) => current?.path === path ? { ...current, revealError: "Couldn’t reveal the exported file." } : current);
              }
            }}>
              Reveal in Finder
            </button>
          )}
          {status.revealError && <span role="alert">{status.revealError}</span>}
          {status.resolveTimeline && (
            <details className="resolve-import-details">
              <summary>Import details</summary>
              {status.resolvePlanChange && <p>{status.resolvePlanChange}</p>}
              <dl>
                <dt>Timeline name in XML</dt>
                <dd>{status.resolveTimeline.name}</dd>
                <dt>Timeline frames</dt>
                <dd>{status.resolveTimeline.timeline_frames}</dd>
                {status.resolveTimeline.source_path && <>
                  <dt>Source media</dt>
                  <dd>{status.resolveTimeline.source_path}</dd>
                </>}
                <dt>Exported XML</dt>
                <dd>{status.path}</dd>
                <dt>XML SHA-256</dt>
                <dd>{status.resolveTimeline.xml_sha256}</dd>
                {status.resolveTimeline.source_fingerprint && <>
                  <dt>Source fingerprint</dt>
                  <dd>{status.resolveTimeline.source_fingerprint}</dd>
                </>}
              </dl>
              <p>After import, confirm the timeline frame count and playback sync.</p>
              <button type="button" className="ghost-btn" onClick={async () => {
                const manifest = status.resolveTimeline?.manifest;
                if (!manifest) return;
                let message = "Manifest copied";
                let showManifest = false;
                try {
                  await navigator.clipboard.writeText(manifest);
                } catch {
                  message = "Select the manifest and use Copy.";
                  showManifest = true;
                }
                setStatus((current) => current?.resolveTimeline?.manifest === manifest
                  ? { ...current, manifestStatus: message, showManifest } : current);
              }}>Copy cut manifest</button>
              {status.manifestStatus && <span role="status">{status.manifestStatus}</span>}
              {status.showManifest && (
                <textarea aria-label="Cut manifest" readOnly rows={5}
                  value={status.resolveTimeline.manifest} onFocus={(event) => event.currentTarget.select()} />
              )}
            </details>
          )}
        </div>
      )}
      <button
        className="export-btn"
        onClick={() => setOpen((v) => !v)}
        disabled={!!blocked || choosingPath || exportMutation.isPending}
        title={blocked}
      >
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6">
          <path d="M8 10V2M5 5l3-3 3 3M3 10v3h10v-3" />
        </svg>
        Export
      </button>
      {open && (
        <div className="export-dropdown">
          {EXPORT_TARGETS.map((t) => {
            const unavailable = t.target === "resolve_xml" ? resolveAudioLimit : undefined;
            return (
            <button
              key={t.target}
              className="export-item"
              onClick={() => void run(t.target, t.ext, t.label)}
              disabled={exportMutation.isPending || !!unavailable}
            >
              {t.label}{unavailable && ` (${unavailable})`}
            </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
