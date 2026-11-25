// Model shapes are GENERATED from the Rust core (one source of truth):
//   TS_RS_EXPORT_DIR="$(pwd)/frontend/src/ipc/generated" \
//     cargo test -p roughcut-core --features ts-bindings export_bindings
// Event payloads and tool-result envelopes below follow docs/tool-api.md.

export type { Media } from "./generated/Media";
export type { Word } from "./generated/Word";
import type { TranscriptSegment as GeneratedTranscriptSegment } from "./generated/TranscriptSegment";
export type TranscriptSegment = GeneratedTranscriptSegment & { speaker?: string };
export type { Transcript } from "./generated/Transcript";
export type { Clip } from "./generated/Clip";
export type { ClipOrigin } from "./generated/ClipOrigin";
export type { Padding } from "./generated/Padding";
export type { Timeline } from "./generated/Timeline";
export type { EditOp } from "./generated/EditOp";
export type { EditAction } from "./generated/EditAction";
export type { ActionSource } from "./generated/ActionSource";
import type { Project as GeneratedProject } from "./generated/Project";
import type { SourceRange } from "./generated/SourceRange";
export interface ResolveTimelineBinding {
  timeline: {
    timeline_name: string;
    start_timecode: string;
    rate_num: number;
    rate_den: number;
  };
  dialogue_stream_index: number;
}

export type Project = GeneratedProject & {
  protected_ranges?: SourceRange[];
  resolve_timeline?: ResolveTimelineBinding;
};
import type { Preferences as GeneratedPreferences } from "./generated/Preferences";
export interface SavedRuleSet {
  name: string;
  custom_filler_words: string[];
  profanity_phrases: string[];
  keyword_phrases: string[];
  never_cut_phrases: string[];
}
export type Preferences = GeneratedPreferences & {
  profanity_phrases?: string[];
  keyword_phrases?: string[];
  never_cut_phrases?: string[];
  saved_rule_sets?: SavedRuleSet[];
};
export type { Aggressiveness } from "./generated/Aggressiveness";
export type { MediaAssets } from "./generated/MediaAssets";
export type { Layout } from "./generated/Layout";
export type { CutSuggestion } from "./generated/CutSuggestion";
import type { CutProposal as GeneratedCutProposal } from "./generated/CutProposal";
export type CutProposal = Omit<GeneratedCutProposal, "cuts"> & { cuts: ProposedCut[]; transcript_id?: string };
import type { ProposedCut as GeneratedProposedCut } from "./generated/ProposedCut";
export type ProposedCut = GeneratedProposedCut & {
  rule_category?: "fillers" | "profanity" | "keywords" | "search";
  min_word_confidence?: number;
  phrase_matches?: { match_id: string; source_in_frame: number; source_out_frame: number }[];
};
export type { HistoryEntry } from "./generated/HistoryEntry";
export type { History } from "./generated/History";
export type { ThumbnailRef } from "./generated/ThumbnailRef";
export type { SavedClip } from "./generated/SavedClip";
export type { SavedClipSummary } from "./generated/SavedClipSummary";
export type { ClipCandidate } from "./generated/ClipCandidate";
export type { ClipFraming } from "./generated/ClipFraming";
export type { ClipEditSession } from "./generated/ClipEditSession";
export type { ClipFormat } from "./generated/ClipFormat";
export type { SourceRange } from "./generated/SourceRange";
export type { PlaySpan } from "./generated/PlaySpan";
export type { ClipBatchError } from "./generated/ClipBatchError";
export type { ClipExportResult } from "./generated/ClipExportResult";
export type { ModelMode } from "./generated/ModelMode";
export type { ModelStatus } from "./generated/ModelStatus";
export type { RouteReason } from "./generated/RouteReason";
export type { KeyCheck } from "./generated/KeyCheck";
export type { HostedKeyResult } from "./generated/HostedKeyResult";

import type { Clip } from "./generated/Clip";
import type { EditAction } from "./generated/EditAction";
import type { ModelMode } from "./generated/ModelMode";
import type { RouteReason } from "./generated/RouteReason";
import type { ProjectSummary as GeneratedProjectSummary } from "./generated/ProjectSummary";
export type ProjectSummary = GeneratedProjectSummary & {
  source_filename?: string | null;
  source_duration?: number | null;
};
import type { Timeline } from "./generated/Timeline";

// ---------------------------------------------------------------------------
// Event payloads (docs/tool-api.md — "Events emitted by the core";
// Rust side: core/src/events.rs CoreEvent)
// ---------------------------------------------------------------------------

export type { ProgressTask } from "./generated/ProgressTask";
import type { ProgressTask } from "./generated/ProgressTask";

export interface ProgressEvent {
  task: ProgressTask;
  project_id?: string;
  fraction: number; // 0..1
  message: string;
}

export type AgentStepKind = "thinking" | "tool_call" | "tool_result" | "final";

export interface AgentStepEvent {
  project_id: string;
  step: number;
  kind: AgentStepKind;
  tool?: string;
  args?: Record<string, unknown>;
  result?: Record<string, unknown>;
  text?: string;
}

export interface TimelineChangedEvent { project_id: string }
export interface TranscriptChangedEvent { project_id: string }

export type AppEventName =
  | "progress" | "agent-step" | "timeline-changed" | "transcript-changed"
  | "projects-changed" | "media-assets-changed" | "confirm-request"
  | "terminal-output" | "terminal-exit" | "clips-changed" | "model-route"
  | "project-opened" | "cut-proposal-changed";

/** `model-route`: where the AI steps actually run changed (Hosted fell back
 *  to Local, came back, or the key was rejected) — the top-bar chip follows
 *  it. `reason` is absent when nothing is wrong. */
export interface ModelRouteEvent {
  mode: ModelMode;
  effective: ModelMode;
  reason?: RouteReason;
}

export interface ConfirmRequestEvent { id: string; summary: string }

/** An MCP client (e.g. Claude Code in the terminal) opened this project. */
export interface ProjectOpenedEvent { project_id: string }

/** A chunk of PTY output: `data` is base64-encoded raw bytes (write to xterm
 *  as a Uint8Array — never as a string, to survive multibyte boundaries). */
export interface TerminalOutputEvent { id: string; data: string }
export interface TerminalExitEvent { id: string; code: number | null }

// ---------------------------------------------------------------------------
// Common tool result envelopes
// ---------------------------------------------------------------------------

export type ExportTarget =
  | "premiere_xml" | "fcp_xml" | "resolve_xml" | "resolve_manifest" | "edl" | "otio" | "mp4" | "srt" | "txt" | "transcript_json";

// Shell-command status shapes are GENERATED (same regime as the model
// types — ADR-0003 extended to the provisioning surface).
export type { ResolveStatus } from "./generated/ResolveStatus";
export type { AlignerStatus } from "./generated/AlignerStatus";
export type { RuntimeStatus } from "./generated/RuntimeStatus";
export type { SetupStatus } from "./generated/SetupStatus";
export type { TierStatus } from "./generated/TierStatus";
export type { SendToResolveOutcome } from "./generated/SendToResolveOutcome";

/** Whisper model tiers (NOT the generated ModelTier preference enum). */
export type WhisperTier = "accurate" | "compact";

// plan_duration_cut (core/src/engine/semantic.rs DurationCutPlan + the
// apply=true receipt from tools.rs)
export interface DurationCutPlan {
  segment_ids: string[];
  before_s: number;
  projected_after_s: number;
  target_s: number;
  method: "centrality" | "heuristic";
  notes: string;
}

export interface DurationCutReceipt {
  applied: number;
  action: EditAction;
  before_s: number;
  included_duration_s: number;
  target_s: number;
  method: string;
  notes: string;
}

export interface EditResult { action: EditAction; timeline: Timeline }
export interface SplitResult { clips: [Clip, Clip]; timeline: Timeline }
export interface UndoResult { action: EditAction | null; timeline: Timeline }
/** generate_rough_cut: what it proposed (or, with `apply`, cut). */
export interface RoughCutResult {
  cut_count: number; seconds: number;
  pauses: number; pause_seconds: number;
  fillers: number; filler_seconds: number;
  words: number; word_seconds: number;
  suggestions: number; audio_checked: boolean; applied: boolean;
  action?: EditAction; note: string;
}
export interface ApplyProposalResult {
  action: EditAction | null; applied: number; skipped: number; seconds: number;
  cut_count: number; included_duration_s: number;
}
export interface InstructionResult { actions: EditAction[]; summary: string }
export interface ExportResult {
  path: string;
  resolve_timeline?: {
    name: string;
    source_path: string | null;
    source_fingerprint: string | null;
    timeline_frames: number;
    xml_sha256: string;
    cut_plan_version: string;
    manifest: string;
  };
}
export interface ListProjectsResult { projects: ProjectSummary[]; trash: ProjectSummary[] }
export interface ToolError { code: string; message: string }
