import type { TranscriptSegment } from "../ipc/types";

export function indexTranscriptPlayback(segments: readonly TranscriptSegment[]) {
  const ordered = [...segments].sort((a, b) => a.start - b.start);
  let furthestEnd = -Infinity;
  const ends = ordered.map((segment) => {
    furthestEnd = Math.max(furthestEnd, segment.end);
    return furthestEnd;
  });
  return (time: number): string | null => {
    let low = 0;
    let high = ends.length;
    while (low < high) {
      const middle = low + Math.floor((high - low) / 2);
      if (ends[middle]! <= time) low = middle + 1;
      else high = middle;
    }
    const segment = ordered[low];
    return segment && segment.start <= time && time < segment.end ? segment.id : null;
  };
}
