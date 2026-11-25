# Resolve transcript editor PRD status

The local desktop workflow follows the request to edit from an imported
transcript without AI or MCP. Evidence for standalone tests and real BTN
media checks is in [Local verification](LOCAL-VERIFICATION.md).

| PRD | Current evidence | Remaining work or verification |
| --- | --- | --- |
| 01 Product scope | Timed JSON and SRT import, transcript selection, cut review, undo and manual XML handoff are wired in source. | Complete a desktop edit and Resolve import using matching media and transcript. |
| 02 Fork and provenance | Fork and pinned upstream revision are recorded in the release manifest. | Record an immutable release candidate. |
| 03 Security hardening | Desktop CSP, local file authorization, bounded byte ranges and staged exports are implemented. Standalone path and response tests passed. Packaged app started with a network-deny policy and no observed Internet sockets. | Verify the packaged runtime and permissions with the desktop test suite. |
| 04 Build and dependencies | The locked macOS build passed offline, including Cargo build scripts. The local bundle passed strict ad hoc signature verification. | Verify the dependency and release artifact receipts. |
| 05 Resolve capability spike | Installed Resolve bundle version and build were read from disk. | Verify edition, manual XML import, media relink and sync in a disposable project while Resolve is idle. |
| 06 Resolve bridge | Manual handoff includes a source-bound XML receipt, unique export ID and timeline name, XML checksum, and a cut manifest. The export panel compares successive plans during the open session. | Verify the handoff in Resolve. Automated Resolve project selection, import and acknowledgement are pending. |
| 07 Local transcription | Imported word-timed JSON and cue-timed SRT are supported. | On-device inference is outside the requested manual workflow. Verify imported timing and selected dialogue track in the desktop editor. |
| 08 Transcript editor UI | Virtualized transcript, playback, explicit edits, corrections, highlights, protected ranges and keyboard controls are wired. | Verify long-transcript interaction, keyboard accessibility and undo after restart. |
| 09 Rules and review | Deterministic phrase search, saved dictionaries and named rule sets, import/export, category selection and project protections are wired. Applied reviews retain rule and match IDs, source frames, available word IDs and minimum confidence, and bounded context, including for matches merged into a single cut. Desktop history loads receipts in pages. Protecting one match revises the proposal to include the remaining unprotected portions, and the core rejects reviews when phrase lists change. | Verify receipt persistence, stable identities across scans, stale review rejection, split and application, and protection end to end. |
| 10 Edit plan and XML | Frame helpers passed standalone 50-cut checks. XML and manifest share retained frame ranges. | Verify actual exported XML and a 50-cut Resolve import against source frames, audio and media hashes. |
| 11 AI and MCP hardening | The desktop path disables AI inference and omits MCP startup. | Optional AI and MCP modes are outside the requested desktop workflow. Verify absence of listeners and inference traffic in the packaged runtime. |
| 12 Test matrix | Standalone Rust tests, focused frontend checks, and a real BTN timed-import, cut, XML and undo integration test passed. | Run the desktop, offline, crash-recovery and Resolve acceptance tests. |
| 13 Installer and release | Release manifest records local data and uninstall behavior. The workflow requires signature and notarization checks before staging the DMG, then verifies its checksum before publication. The frontend SBOM and Rust dependency inventory commands ran locally. | Produce a signed installer, verify the actual release artifacts and dependency notices, and test installation on a clean Mac. |
| 14 Optional Studio panel | Standalone editor uses manual XML handoff. | The optional Studio panel has no implementation or runtime acceptance evidence. |
| 15 Roadmap and gates | Verification and release records identify open gates. | Resolve and release gates remain open. |
| 16 Agent runbook | Local verification notes cover the authorized test path and current limits. | Keep the receipts with the exact candidate that is built and tested. |
| 17 License and commercial | Baseline license and dependency provenance are recorded. | Review distribution terms and prepare notices for the exact packaged artifact before redistribution. |
| 18 Architecture contract | Rust owns project, transcript, timeline and export state. The frontend uses desktop IPC. | Verify all critical boundaries and persistence behavior in the running desktop app. |
| 19 Release checklist | Release manifest tracks source, evidence and distribution requirements. | Desktop and Resolve acceptance, artifact receipts and release approval are pending. |

### Editable Resolve timelines

The completed single-file test does not verify the multicam workflow. The
selected existing or newly created Resolve timeline is the edit target. A
matching video or dialogue export supplies playback and transcript timing.
Bind the cut plan to the selected timeline identity, start frame, frame rate,
and edit state, then apply reviewed cuts to a duplicate. Reject stale or
mismatched plans and defer all Resolve operations during rendering.

The installed Resolve scripting declarations expose timeline duplication,
render-status checks, and deletion of whole timeline items. They do not
document a split operation for arbitrary frame ranges. Before enabling an
automatic apply action, verify an edit path on a disposable BTN multicam
timeline. Compare retained angle switches, linked audio, additional tracks,
and cut boundaries in Resolve, and confirm the original timeline is untouched.
