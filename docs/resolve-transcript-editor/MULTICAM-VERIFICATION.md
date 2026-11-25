# Resolve multicam verification

## Offline review

Choose **Open Resolve review** in RoughCut and select a local JSON binding
followed by the full-duration video export. Import the matching timed JSON or
SRT in Transcript, then review edits with the existing transcript and timeline
controls. **Copy Resolve cut plan** returns the source binding, half-open
ranges in original Resolve frames, the expected output length and a stable
plan SHA-256. Resolve operations are still manual.

The binding file has a `timeline` object and three top-level fields
`playback_sha256`, `transcript_sha256`, and `dialogue_stream_index`. Supply
lowercase SHA-256 digests of the exact file bytes. The dialogue index uses
FFprobe's stream indices across the playback file. Transcript time zero must
be the first frame of the export.

The `timeline` object has `schema_version` set to 1, stable Resolve
`project_id` and `timeline_id`, `timeline_name`, `revision_sha256`,
`start_frame`, exclusive `end_frame`, `start_timecode`, integer
`rate_num` and `rate_den`, and boolean `drop_frame`. Use the integer
Resolve clock and exact rate from the source timeline. For drop frame use
a semicolon before the frame field.

RoughCut treats a binding as local provenance supplied by the editor.
It verifies the files and clock against those declared values. A future
writer needs a fresh Resolve revision and idle check before using a plan.
The desktop workflow currently accepts an externally prepared binding and
does not generate one from Resolve.

## BTN Episode 2 native spike

Resolve 21.1.0, build 21.1.00017, was closed before the spike. Its Deliver
page showed an empty render queue after importing the archive into a
separate local project named **RoughCut BTN Multicam Verification**. The
archive copy and the SSD original had identical SHA-256 digests.

- Source timeline **S03E02 Recovered - Multicam edit** used a zero start
  and 30000/1001 fps. The displayed original duration was `00:17:04:29`.
  The timeline contained a three-angle multicam item on V1 and the external
  Recording 304 dialogue on A1. Resolve loaded the camera footage and drew
  the dialogue waveform.
- Exporting and reimporting a native DRT into the disposable project retained
  the multicam item, its three nested angle timelines and the dialogue WAV.
  The imported timeline opened at the same length as the source.
- A marked-range Ripple Delete alone removed the whole picture item. That
  test was undone on the copy. Unlocking A1, splitting both tracks explicitly
  and ripple deleting only the two isolated middle items worked for this
  fixture. Track selection and lock state affected which items split.
- The result DRT records the same 312-frame removal `[1796, 2108)` on both
  tracks. Both retained right-hand items start at output frame 1796 with
  source in at frame 2108. V1 shortened from 30,738 to 30,426 frames.
  A1 shortened from 30,749 to 30,437 whole frames with its subframe tail
  preserved. The 11-frame difference between track ends was present before
  the cut.
- The untouched source still showed `00:17:04:29` when reopened. A retained
  segment in the edited copy switched from Lily to Ruth in Resolve. The
  pre-switch result was exported and reopened as **RoughCut BTN Reopened
  Cut**. The retained multicam item still showed the Lily angle.

The two local evidence exports are `/private/tmp/RoughCut BTN DRT Roundtrip.drt`
and `/private/tmp/RoughCut BTN DRT After Split Ripple.drt`. The native DRT
export retained three nested angle timelines in both archives. No application
code edits DRT internals.

## Remaining acceptance

The installed scripting declarations document `DuplicateTimeline`,
`IsRenderingInProgress`, `DeleteClips` and timeline interchange. They
provide no arbitrary-frame split command. The manual native spike establishes
a path for one within-angle removal. A supported automated writer and its
stale, interrupted and repeated apply behavior remain unverified.

The recovered BTN fixture has separate dialogue and a single selected angle
in the source edit. A cut spanning an existing angle switch, linked audio,
additional tracks, titles and effects, and a newly assembled multicam still
need verification. No transcript and playback export were established for
this exact revision. The Episode 10 single-file timing pair therefore was
not used for multicam verification.

## Build and regression checks

The locked macOS release build, frontend bundle, three multicam tests and
14 desktop tests passed with networking denied. Focused TypeScript checks
passed for the editor entry points touched by the review feature. The real
BTN Episode 10 timed import, cut, Resolve XML and undo regression also
passed under the network restriction. That fixture verifies the existing
single-file workflow.
