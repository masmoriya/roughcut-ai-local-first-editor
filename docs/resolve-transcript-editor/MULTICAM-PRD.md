# PRD: Edit Resolve multicam timelines by transcript

## Outcome

An editor opens an existing or newly assembled Resolve multicam timeline in RoughCut, selects spoken words or phrases, and ripple removes those ranges from a new editable Resolve timeline. A rendered video or dialogue mix supplies synchronized playback and transcript timing. Resolve remains the authority for angles, tracks, and the timeline.

## Workflow

1. Choose a specific Resolve project and timeline, including a multicam just created in Resolve. Show its name, start timecode and frame rate.
2. Attach a playback reference from that exact timeline revision and import its timed JSON or SRT. Confirm the timing origin and dialogue track. Show whether selection works at word or caption level. Refuse mismatched, stale or ambiguous references.
3. Select transcript text or search for phrases such as “um.” Preview the precise removals, adjust boundaries, exclude individual matches, and undo or redo pending edits.
4. Apply the reviewed cuts to a clearly named duplicate. Preserve retained angle switches, angle editability, synchronized audio, additional tracks, and supported titles and effects. Identify any unsupported feature before writing. Show the resulting timeline and the outcome of verification.

## Requirements

- Support both already edited multicams and newly assembled multicams. Camera syncing and angle creation stay in Resolve.
- Resolve owns timeline identity and frame coordinates. Bind project and timeline IDs, edit revision, start frame, rational frame rate, drop-frame display convention, and reference identity. Translate transcript time into half-open integer frame ranges; merge overlaps once and apply against the original timeline coordinates.
- Recheck the source revision and render status before any operation against Resolve and before mutation. Defer when rendering or when status cannot be established. Leave the source timeline untouched. A stale plan requires rebinding or review.
- Make retries idempotent. Keep a local receipt with the source and result identities, accepted cut ranges, verification result, and failure state. Failed or partial results must be clearly distinguishable and recoverable.
- Keep media, transcripts, logs, and interchange local. No AI, MCP, cloud dependency or network listener. Scope file access to approved local paths. Treat imported files as data, bound their size, and reject external XML entities and remote references. Avoid transcript text in diagnostic logs.

## Acceptance

- On a disposable copy of a real BTN multicam, remove passages within an angle, across a switch, and across multiple audio and video tracks. Also verify a newly assembled Resolve multicam.
- Compare expected and actual frame counts and boundaries, including nonzero timeline starts and fractional rates. Confirm retained angles can still be switched, source media relinks, and dialogue stays synchronized. Check titles and effects on untouched ranges and refuse unsupported intersecting edits before apply.
- Verify original edit state, reopen the result, and cover stale snapshots, mismatched exports, overlapping ranges, repeat apply, interrupted apply, and render deferral. Confirm offline runtime operation.
- Markers and flattened XML may aid diagnosis but do not count as editable multicam acceptance.
