// Load timeline view assets (real waveform peaks + thumbnail filmstrip).
// The core hands us file paths; the binary flows over the scoped media protocol
// (fetch on a convertFileSrc URL), never JSON IPC.

import { mediaSrc } from "../ipc/api";
import type { MediaAssets } from "../ipc/types";

export interface LoadedThumb {
  time: number;
  img: HTMLImageElement;
}

export interface TimelineAssets {
  /** u8 peaks, `pps` per second of source time. */
  peaks: Uint8Array | null;
  pps: number;
  thumbs: LoadedThumb[];
}

function loadImage(url: string): Promise<HTMLImageElement | null> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = url;
  });
}

async function fetchPeaks(url: string): Promise<Uint8Array | null> {
  try {
    const response = await fetch(url);
    return response.ok ? new Uint8Array(await response.arrayBuffer()) : null;
  } catch {
    return null;
  }
}

const pendingPeaks = new Map<string, Promise<Uint8Array | null>>();

function loadPeaks(path: string | null): Promise<Uint8Array | null> {
  const url = path ? mediaSrc(path) : null;
  if (!url) return Promise.resolve(null);
  const pending = pendingPeaks.get(url);
  if (pending) return pending;
  const request = fetchPeaks(url).finally(() => { pendingPeaks.delete(url); });
  pendingPeaks.set(url, request);
  return request;
}

export async function loadTimelineAssets(
  meta: MediaAssets,
  onPeaks?: (peaks: Uint8Array) => void,
): Promise<TimelineAssets> {
  const thumbs: LoadedThumb[] = [];
  const [peaks, loaded] = await Promise.all([
    loadPeaks(meta.peaks_path).then((peaks) => {
      if (peaks?.length) onPeaks?.(peaks);
      return peaks;
    }),
    Promise.all(meta.thumbnails.map(async (t) => {
      const url = mediaSrc(t.path);
      if (!url) return null;
      const img = await loadImage(url);
      return img ? { time: t.time, img } : null;
    })),
  ]);
  for (const t of loaded) if (t) thumbs.push(t);
  thumbs.sort((a, b) => a.time - b.time);

  return { peaks, pps: meta.peaks_per_second, thumbs };
}
