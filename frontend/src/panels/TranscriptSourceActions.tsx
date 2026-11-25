import { useEffect, useState } from "react";
import { useStore } from "@tanstack/react-store";
import { getSetupStatus, isTauri } from "../ipc/api";
import type { Media, Project } from "../ipc/types";
import {
  dialogueStreams,
  startTranscription,
  transcribeErrors,
  transcriptionJobs,
} from "../state/transcribeStore";
import { ImportTimedTranscript } from "./ImportTimedTranscript";

export function TranscriptSourceActions({ project, media }: { project: Project; media: Media }) {
  const job = useStore(transcriptionJobs, (jobs) => jobs[project.id]);
  const error = useStore(transcribeErrors, (errors) => errors[project.id]);
  const [modelInstalled, setModelInstalled] = useState<boolean>();
  const [stream, setStream] = useState("");
  const streams = dialogueStreams(media);
  const locallyTranscribable = isTauri && !project.resolve_timeline && !project.clip_session && streams.length > 0;

  useEffect(() => {
    if (!locallyTranscribable || job) return;
    let active = true;
    void getSetupStatus().then(
      (status) => { if (active) setModelInstalled(status.transcription_model_installed); },
      () => { if (active) setModelInstalled(undefined); },
    );
    return () => { active = false; };
  }, [job, locallyTranscribable]);

  if (job) {
    return (
      <div className="transcribe-progress">
        <p className="transcribe-message" role="status">{job.message}</p>
        <progress aria-label="Transcription progress" max={1} value={job.fraction || undefined} />
      </div>
    );
  }

  return (
    <>
      {error && <p role="alert">{error}</p>}
      {locallyTranscribable ? (
        <>
          {streams.length > 1 && (
            <label>Dialogue track <select value={stream} onChange={(event) => setStream(event.target.value)}>
              <option value="">Choose a track</option>
              {streams.map((audio, index) => (
                <option key={audio.index} value={audio.index}>
                  Track {index + 1}{audio.language && ` · ${audio.language}`}
                </option>
              ))}
            </select></label>
          )}
          <button className="ghost-btn" disabled={streams.length > 1 && stream === ""}
            onClick={() => void startTranscription(project.id, stream === "" ? undefined : Number(stream), modelInstalled === false)}>
            {modelInstalled === false ? "Download model and transcribe" : "Transcribe on device"}
          </button>
        </>
      ) : <p role="status">No transcript yet.</p>}
      <ImportTimedTranscript projectId={project.id} sourcePath={media.file_path} mediaId={media.id}
        boundDialogue={project.resolve_timeline?.dialogue_stream_index} audioStreams={streams} />
    </>
  );
}
