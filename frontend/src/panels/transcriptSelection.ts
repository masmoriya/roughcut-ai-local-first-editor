export function selectedTranscriptWords(container: HTMLElement | null) {
  const selection = window.getSelection();
  if (!container || !selection || selection.isCollapsed || !selection.rangeCount) return null;
  const range = selection.getRangeAt(0);
  if (!container.contains(range.startContainer) || !container.contains(range.endContainer)) return null;

  let start = Infinity;
  let end = -Infinity;
  let count = 0;
  const segments = new Set<HTMLElement>();
  for (const word of container.querySelectorAll<HTMLElement>("[data-wstart][data-wend]")) {
    if (!range.intersectsNode(word)) continue;
    const text = word.firstChild;
    if (text === range.startContainer && range.startOffset === text?.textContent?.length) continue;
    if (text === range.endContainer && range.endOffset === 0) continue;
    start = Math.min(start, Number(word.dataset.wstart));
    end = Math.max(end, Number(word.dataset.wend));
    const segment = word.closest<HTMLElement>("[data-segid]");
    if (segment) segments.add(segment);
    count++;
  }
  return count && Number.isFinite(start) && Number.isFinite(end) && end > start
    ? { start, end, count, crossesParagraphs: segments.size > 1,
        segmentId: segments.size === 1 ? segments.values().next().value?.dataset.segid : undefined }
    : null;
}

export function selectBetweenWords(anchor: HTMLElement, word: HTMLElement): void {
  const selection = window.getSelection();
  if (!selection || !anchor.isConnected || !word.isConnected) return;
  const forward = anchor === word ||
    Boolean(anchor.compareDocumentPosition(word) & Node.DOCUMENT_POSITION_FOLLOWING);
  const first = forward ? anchor : word;
  const last = forward ? word : anchor;
  const range = document.createRange();
  range.setStartBefore(first);
  range.setEndAfter(last);
  selection.removeAllRanges();
  selection.addRange(range);
}
