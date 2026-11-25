import { Store } from "@tanstack/react-store";
import { listen } from "@tauri-apps/api/event";
import { callTool, getSetupStatus, installTranscriptionModel, isTauri } from "../ipc/api";
import type { Media, ProgressEvent, Project } from "../ipc/types";
import { errText } from "../panels/clipUtil";

export const transcribeErrors = new Store<Record<string, string>>({});

/** Set (or clear, with null) a project's transcription error. */
export function setTranscribeError(projectId: string, message: string | null): void {
  transcribeErrors.setState((all) => {
    const next = { ...all };
    if (message === null) delete next[projectId];
    else next[projectId] = message;
    return next;
  });
}

type TranscriptionProgress = Pick<ProgressEvent, "fraction" | "message">;

export const transcriptionJobs = new Store<Record<string, TranscriptionProgress>>({});

export function dialogueStreams(media: Media) {
  return (media as Media & {
    audio_streams?: { index: number; language?: string | null }[];
  }).audio_streams ?? [];
}

export async function startTranscription(projectId: string, stream?: number, installModel = false) {
  if (!isTauri || transcriptionJobs.state[projectId]) return;
  const update = (progress: TranscriptionProgress) => {
    transcriptionJobs.setState((jobs) => ({ ...jobs, [projectId]: progress }));
  };
  update({ fraction: 0, message: installModel ? "Preparing speech model…" : "Starting transcription…" });
  setTranscribeError(projectId, null);
  let stopListening = () => {};
  try {
    stopListening = await listen<ProgressEvent>("progress", ({ payload: event }) => {
      if (event.task === "transcribe" && event.project_id === projectId && event.fraction < 1) {
        update(event);
      } else if (installModel && event.task === "model_download") {
        update({ fraction: event.fraction, message: "Downloading speech model…" });
      }
    });
    if (installModel) {
      await installTranscriptionModel();
      update({ fraction: 0, message: "Starting transcription…" });
    }
    await callTool("transcribe", {
      project_id: projectId,
      audio_stream_index: stream,
    });
  } catch (error) {
    setTranscribeError(projectId, errText(error));
  } finally {
    stopListening();
    transcriptionJobs.setState((jobs) => {
      const next = { ...jobs };
      delete next[projectId];
      return next;
    });
  }
}

export async function autoTranscribe(project: Project) {
  if (!isTauri || !project.media || dialogueStreams(project.media).length !== 1) return;
  try {
    if ((await getSetupStatus()).transcription_model_installed) {
      await startTranscription(project.id);
    }
  } catch (error) {
    setTranscribeError(project.id, errText(error));
  }
}
