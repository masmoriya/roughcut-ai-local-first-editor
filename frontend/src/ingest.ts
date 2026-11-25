// Attach the source and start local transcription when the model is ready.

import { callTool } from "./ipc/api";
import type { Project } from "./ipc/types";
import { autoTranscribe } from "./state/transcribeStore";

export function baseName(path: string): string {
  const last = path.split(/[\\/]/).pop() ?? path;
  return last.replace(/\.[^.]+$/, "") || "Untitled project";
}

export interface IngestCallbacks {
  /** Coarse phase labels; fine-grained progress arrives via `progress` events. */
  onPhase?: (message: string) => void;
}

export async function ingestFile(
  name: string,
  filePath: string,
  cb: IngestCallbacks = {},
): Promise<string> {
  cb.onPhase?.("Creating project…");
  // create_project(file_path) probes and attaches in one step — do NOT call
  // import_media after this; that probes a second time and replaces media.
  const project = await callTool<Project>("create_project", {
    name,
    file_path: filePath,
  });
  void autoTranscribe(project);
  return project.id;
}
