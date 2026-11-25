# Local verification

The desktop workflow is local timed transcript import, transcript selection and
non-destructive timeline edits, followed by manual Resolve XML import. The
desktop shell has no MCP startup, embedded shell, hosted-model settings,
or automatic Resolve integration.

## Evidence

| Area | Verified | Pending |
| --- | --- | --- |
| Provenance | GitHub fork and pinned upstream commit. Drive package copied into the checkout. | Release and distribution checks. |
| Source | Changed Rust and TypeScript files parsed. The Apple Silicon Cargo graph resolves offline with the lockfile. Locked frontend dependencies installed with lifecycle scripts disabled. Strict TypeScript checks passed for the edited desktop panels and their imported modules. A direct esbuild bundle of the desktop entry point excluded the model store and browser mock API. | Native Rust compilation, the packaged frontend, and execution of the desktop test suites. |
| Real media | BTN S03E02 MOV metadata and two seconds of audio decoded using the local input restrictions. The timed JSON was checked against its originating dialogue WAV. | Verify imported word timings, playback and transcript editing in the desktop app. |
| Desktop security | Source changes limit media reads and disable external inference and script installation entry points. | Verify the CSP and media path restrictions in the built desktop app. |
| Resolve | XML writer uses a shared frame range for video and audio and the export path refuses overwrites. | Verify import into a new disposable timeline, relinked media, cut positions and audio sync. |

## Target machine

- Installed Resolve bundle reports version 21.1.0, build 21.1.00017.
  Edition and current render activity are unverified. The Resolve process
  was open during inspection. The check only read installed bundle metadata.
- BTN S03E02 has one 48 kHz stereo audio track and 30000/1001 video.
  Six standalone Rust frame tests passed, including 500 cut sets across
  24, 25, 29.97, 30 and 59.94 fps. The 50-cut cases covered 24,
  25, 29.97 and 30 fps using the frame calculation shared by XML and its
  manifest.
- Standalone XML text sanitization and file URL encoding tests passed.
- A standalone concurrent publication test on the internal filesystem
  preserved the winning export byte for byte. The same test passed in a
  disposable directory on the ExFAT BTN SSD, exercising publication on a
  volume without hard links. A follow-up probe confirmed the temporary source
  remained after publication and the volume rejected hard links, identifying
  the exclusive-copy path. Temporary files were removed afterward.
- The frontend entry point bundled with esbuild without a project script.
  The desktop app itself has yet to be compiled or launched.
- The standalone source timecode test passed for 29.97 and 59.94 drop-frame
  boundaries, invalid frame labels, and the zero-based BTN source label.
- The frontend lockfile audit reports zero findings. The last Apple Silicon
  OSV check reports nine ZIP advisories in the optional alignment dependency
  graph and one unmaintained-crate advisory for paste. Offline locked Cargo
  resolution for the current Apple Silicon desktop build excludes ZIP, paste,
  Whisper, Candle and wav2vec2. The earlier results still apply to builds that
  enable alignment. An OSV batch query for all 317 registry packages in the
  current locked Apple Silicon desktop graph returned zero advisories.

## Acceptance sequence

| Gate | Current evidence | Remaining verification |
| --- | --- | --- |
| G0 | Fork checked out at baseline `224753a`. Drive PRDs are in the local roadmap directory. | Record release-specific attribution and redistribution decisions before packaging. |
| G1 | Local media allowlist, offline editing boundary, frontend bundle and lockfile checks. | Build and exercise the desktop security boundaries. Resolve the remaining dependency advisories or document release exclusions. |
| G2 | Resolve is installed. XML and manifest share frame calculations. Read-only BTN source probes passed. | Confirm Resolve is idle, then import as a new timeline in a disposable project and compare cuts and audio. |
| G3 | Local transcript import, word edits, phrase lists, speaker filters, protected ranges and review controls are implemented. Reviewed apply persists a revised selection when it encounters newly protected cuts. | Run the desktop workflow through save, restart and undo with real media. Verify that a protection change during review leaves the timeline untouched until the revised selection is confirmed. |
| G4 | Standalone timecode tests and the four-rate frame matrix passed. | Generate and import XML fixtures at all four rates. Verify retained endpoints, original-media hashes and audio sync. |
| G5 | Local source changes only. | Build, signing, notarization, install, rollback and release acceptance are pending. |

The BTN S03E02 source reports one stereo PCM audio stream (stream index 1),
so it can exercise the single-track Resolve XML path. Two read-only FFmpeg
samples totaling 150 seconds had no silence at −35 dB lasting 0.8 seconds.
That threshold check is independent of the editor’s adaptive pause review.
In a three-second video packet sample near source time 300 seconds, all 90
packets reported a duration of 0.033367 seconds. Near 3100 seconds, all 88
sampled packets reported a duration of 1001 stream ticks, consistent with
the same cadence. A full-file FFprobe pass then found exactly 95,069 video
packets and a duration of 1001 stream ticks on every packet, consistent with
the frame count used by the BTN test. Imported cut boundaries still need
verification.

Two-second FFmpeg filter runs on BTN S03E02 produced identical SHA-256 audio
hashes for the default audio selector and explicit stream index 1. The probe
ran without a media output file. A multi-track source and the desktop render
path still need verification.
An explicit stream-1 preview remux of a two-second BTN excerpt produced a
probeable MP4 with H.264 video and stereo AAC. This verifies the FFmpeg
container and codec arguments on that source. Desktop playback and switching
between distinct audio streams still need verification.
Read-only probes of BTN S03E03 and S03E03A also found one stereo PCM stream
each. An earlier S03E05 probe could not read its MOV index. A later
read-only probe found matching nominal and average rates of 60000/1001
and a video and container duration of 1488.887400 seconds. E02, E03 and
E03A also report matching nominal and average rates of 30000/1001.
A full E05 packet scan found 89,244 video packets of 1001 stream ticks each.
E05 V2 has one stereo 48 kHz PCM audio stream at index 1 and starts at
timecode 00:00:00:00. It fits the single-track Resolve XML test case at
59.94 fps.
The E05 transcript marked “Complete external audio” reaches timecode
02:24:16:25, far beyond the V2 video's 24:48.887 duration. It is unsuitable
as the timed transcript for that export.
E03 and E03A have separate dialogue WAVs and timed JSONs with six and five
mapping entries, respectively. Five in-range E03 midpoint comparisons at the
mapped `source` positions against E03 V2 yielded correlations between
-0.027 and 0.027. The raw mapping is unsuitable for that video.
For E03A V2, five midpoint comparisons at the raw offsets and five at the
mapped `timeline` positions all had absolute correlation below 0.030. The
mapped `source` positions yielded no comparable V2 samples. Neither tested
clock establishes E03A V2 as a paired fixture.
The existing BTN episode-02 timed JSON has 154 segments and 10,584 words.
Its final segment ends at 3653.63 seconds, beyond the 3172.14-second V2
media export. These files describe different timelines and are unsuitable
as a paired transcript-import fixture.
All 10,584 word intervals are within their enclosing segment to the
importer's one-millisecond tolerance. The JSON uses `word` for token text
and omits confidence. Its blocking difference is the time mapping.
The local S03E02 program WAV runs 3784.247146 seconds. Its duration can
contain the transcript but does not establish that the two share a clock.
The individual program WAVs are shorter than the full transcript.
The episode-02 JSON identifies the episode and frame rate, but carries no
source filename, duration or fingerprint. It includes ten mapping entries
with separate offsets, source positions and timeline positions. The first
maps offset zero to timeline time 499.5323667 seconds. The offset ranges are
contiguous and end at 3654.85125 seconds, just after the last transcript
segment at 3653.63. The final range starts at offset 3002.232625 but maps to
source time 3120.7176. This supports a concatenated-audio clock for the
transcript. Confirm the mapping against the originating media before
converting it into source-relative edit points. Nine segments and three
individual words straddle mapping boundaries. A conversion must split or
flag those intervals so a single transcript cut cannot span an unrelated
gap in the program recording.
The adjacent `episode-02-dialogue.wav` is mono at 16 kHz and runs exactly
3654.851250 seconds, matching the concatenated offset total. It is the
candidate for verifying the transcript's native clock. Read-only FFmpeg
comparisons used two-second samples at the midpoint of all ten ranges,
resampled both inputs to 16 kHz mono, and compared `offset + duration / 2`
in the dialogue WAV to `source + duration / 2` in the program WAV. Mean
normalized correlation ranged from 0.869 to 1.000, supporting the source
mapping at the sampled positions. The same comparison against V2 audio at
the nine source positions within the video produced correlations between
-0.032 and 0.032. The program WAV mapping does not align V2 at those points.
FFprobe lists video, PCM audio and a data stream in the V2 MOV, with no
subtitle stream. The local SSD search found the separate Review Notes text
and mapped dialogue JSON, but no matching SRT or VTT for the V2 export.
The saved Resolve validation report lists a separate S03E02 Review Notes V2
timeline at 3190.32 seconds with 858 cues. It is also longer than the V2 media
export by 18.18 seconds and should be checked against its own source timeline.

For external timed JSON, supply a top-level `segments` array. Each segment
requires `start` and `end` in source-relative seconds and `text`. Optional
`words` entries use the same fields (`word` is also accepted in place of
`text`) and may include `confidence` from 0 to 1. The optional top-level `fps`
and `source_duration_seconds` are checked against the attached media. Confirm
that the JSON uses the clock of the exact export, and choose the matching
dialogue track if the source contains multiple audio streams.

SRT import uses cue timestamps for paragraph-level edits. A standalone
parser check recovered all 154 episode-02 cues from an SRT stream derived
from the BTN JSON. This tested parsing on the dialogue clock. The V2 video
still needs its own matching transcript for desktop acceptance.

A read-only standalone Rust test used the production media response
function with the Episode 02 V2 MOV. It compared 128-byte responses at the
start, midpoint and end against direct reads from the file, checked the
reported byte ranges and no-store header, and verified that a suffix HEAD
request returned the expected length and no body. A separate test checked
range bounds for large suffixes and offsets. The harness used the locked
HTTP and percent-encoding dependencies and opened only that fixture.
Desktop allowlisting, WebView seeking and Resolve were outside this test.

The installed macOS Resolve application bundle reports version 21.1.0
(build 21.1.00017). The Resolve process was running during this read-only
check. Bundle metadata does not establish license edition, render state,
or XML import compatibility. No Resolve controls or scripting APIs were used.

1. Build the desktop app with the reviewed toolchain and run the focused core
   export and media-access tests.
2. In the desktop app, import an approved BTN clip and timed JSON whose
   timestamps match that exact media export. Verify playback and word seeks
   against the audio, then exercise word selection, search review, correction,
   and undo after closing and reopening the project. Open a cut proposal,
   add a never-cut phrase that matches one of its selected cuts, and apply.
   Confirm that the protected cut is deselected and the timeline is unchanged.
   Review and apply the remaining cuts, then undo the batch.
   Export each saved phrase list as text and JSON. Reimport both, verify
   the draft matches the saved entries, then save and reopen the app to
   confirm persistence. Try malformed JSON and an oversized list, and
   confirm the last saved preferences remain in effect.
   Select a retained transcript passage, choose Add to clip, then save it
   from the draft bar. Reopen it from Clips and compare playback with the
   selected source passage.
   For transcript corrections, open a form and update the same segment's
   speaker through another editor session. Confirm the first form rejects
   its stale save. Reopen the form, change a timed word, then verify the
   original segment and word text survive another correction and a project
   reopen. Export SRT and TXT after cutting one word from the segment.
   Confirm both exports omit that word and preserve the retained correction.
3. Export Resolve XML to a new file and copy its cut manifest from Import
   details. Verify the XML file's SHA-256 against the manifest. Compare the XML
   sequence duration in frames with the manifest's timeline_frames and the
   export receipt. Compare the receipt's source fingerprint and path with
   the manifest. Try exporting again to the same destination and verify
   that the existing file remains byte-for-byte unchanged.
4. When Resolve is idle, import into a disposable project as a new timeline.
   Compare source in/out frames, total frames and audio across the cuts.
   Record Resolve version and edition alongside the result.

The XML currently references source audio track 1. The manifest records the
transcript's FFmpeg stream index, which is an index across all media streams.
Before enabling multitrack XML, verify Resolve's track and channel mapping
with distinguishable audio on each source track. Confirm that the imported
timeline plays the track used for transcript editing across every cut.

The manifest source fingerprint on macOS hashes the file size, modification
time, device and inode. This checks file identity without reading the full
media on every edit. The Resolve acceptance test still needs a full source
SHA-256 before and after export and import to verify that the bytes stayed
unchanged.

### Native build and BTN Episode 10 smoke test

The locked macOS release build, including Cargo build scripts and the frontend
production build, ran under a macOS sandbox that denied network access. The
resulting local app bundle was ad hoc signed and passed strict signature
verification. The packaged process started under the same network restriction
and had no Internet sockets in the process inspection.

The real-media integration test used BTN Episode 10's original
`BTN_S03E03a_edited_full.mov` and `keep-02.json` from the Extreme SSD. The
transcript's reference duration was 2403.501104 seconds. The export's video
stream duration was 2403.501100 seconds at 30000/1001 fps. The test exercised
local media probing, timed transcript import with words, a transcript-driven
cut, Resolve XML export, and undo through the desktop tool dispatcher. It used
a temporary project library and export directory. Resolve was not opened and
playback and timeline import remain unverified in the editor UIs.
