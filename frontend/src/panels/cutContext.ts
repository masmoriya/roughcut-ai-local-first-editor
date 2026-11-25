import type { TranscriptSegment } from "../ipc/types";

export function cutContext(segments: readonly TranscriptSegment[], start: number, end: number) {
  const before: string[] = [];
  const after: string[] = [];
  for (const segment of segments) {
    if (segment.is_silence || segment.end < start - 3 || segment.start > end + 3) continue;
    const words = segment.words.length ? segment.words : [segment];
    for (const word of words) {
      if (word.end <= start && word.end >= start - 3) before.push(word.text);
      if (word.start >= end && word.start <= end + 3) after.push(word.text);
    }
  }
  return {
    before: Array.from(before.join(" ").trim()).slice(-100).join(""),
    after: Array.from(after.join(" ").trim()).slice(0, 100).join(""),
  };
}
