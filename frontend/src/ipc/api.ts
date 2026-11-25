// Desktop IPC entry point for the local editor.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppEventName, Project, SetupStatus } from "./types";

export const isTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function callTool<T = unknown>(
  name: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (isTauri) {
    return invoke<T>("call_tool", { name, args });
  }
  throw new Error("Open the desktop app to edit your timeline.");
}

export function openResolveReview() {
  return invoke<Project | null>("open_resolve_review");
}

export function getResolveCutPlan(projectId: string) {
  return invoke("resolve_cut_plan", { projectId });
}

/** Subscribe to a core event. Returns an unsubscribe function. `onReady`
 *  runs once the listener is live (Tauri registers it asynchronously): read
 *  initial state there, so no event can fall between the read and the
 *  subscription. */
export function onAppEvent<T>(
  event: AppEventName,
  handler: (payload: T) => void,
  onReady?: () => void,
): () => void {
  if (isTauri) {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<T>(event, (e) => handler(e.payload)).then((fn) => {
      if (cancelled) fn();
      else {
        unlisten = fn;
        onReady?.();
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }
  return () => {};
}



export type MediaSetupStatus = Pick<SetupStatus, "ffmpeg" | "ffmpeg_path"> & {
  ffprobe_path: string | null;
  resolve_installed: boolean | null;
  transcription_model_installed: boolean;
};

export async function getSetupStatus(): Promise<MediaSetupStatus> {
  return invoke<MediaSetupStatus>("setup_status");
}

export function installTranscriptionModel(): Promise<void> {
  return invoke("install_transcription_model");
}

// ---- The LOCAL / HOSTED switch ----
// Shell commands, not registry tools (ADR-0004): only the user, in the app,
// turns Hosted on or handles the key. Errors reject with
// `{ error: { code, message } }` (codes include hosted_no_key,
// hosted_not_consented, hosted_key_rejected, invalid_argument).
import type { HostedKeyResult, KeyCheck, ModelMode, ModelStatus } from "./types";

/** In memory: no network, no key-store read. */
export async function getModelStatus(): Promise<ModelStatus> {
  return invoke<ModelStatus>("model_status");
}

/** Hosted needs a stored key, and `consent: true` the first time. */
export async function setModelMode(mode: ModelMode, consent: boolean): Promise<ModelStatus> {
  return invoke<ModelStatus>("set_model_mode", { mode, consent });
}

/** One of claude-opus-5-5 | claude-sonnet-5-5 | claude-haiku-5-5. */
export async function setHostedModel(model: string): Promise<ModelStatus> {
  return invoke<ModelStatus>("set_hosted_model", { model });
}

/** Save (checked first, no tokens spent) or forget (null; also switches to
 *  Local) the Anthropic API key. */
export async function setHostedKey(key: string | null): Promise<HostedKeyResult> {
  return invoke<HostedKeyResult>("set_hosted_key", { key });
}

/** Check the stored key against Anthropic again (Setup's "Test"). */
export async function checkHostedKey(): Promise<KeyCheck> {
  return invoke<KeyCheck>("check_hosted_key");
}

/** Reveal a file in Finder (desktop only). */
export async function revealPath(path: string): Promise<void> {
  if (!isTauri) return;
  return invoke("reveal_path", { path });
}

/** The app log's file on this machine: null in the browser, or if the app
 *  couldn't open it. */
export async function logPath(): Promise<string | null> {
  if (!isTauri) return null;
  return invoke<string | null>("log_path");
}

/** A URL for a small local file read whole: thumbnails, waveform peaks.
 *  Served by the same scoped protocol as playback. */
export function mediaSrc(filePath: string): string | null {
  return localFileUrl(filePath);
}

/** A URL for a video or audio file the preview's <video>/<audio> plays.
 *  Served by the shell's `rcmedia` protocol, which answers each byte range
 *  in full: the asset protocol stops at 1000 KiB per read, and WebKit then
 *  drops the picture of a long recording (its frame tables are bigger). */
export function playbackSrc(filePath: string): string | null {
  return localFileUrl(filePath);
}

function localFileUrl(filePath: string): string | null {
  if (!isTauri) return null;
  // A bare file name cannot be resolved by the scoped media protocol.
  if (!filePath.includes("/") && !filePath.includes("\\")) return null;
  return convertFileSrc(filePath, "rcmedia");
}
