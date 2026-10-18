// Pure helpers for the Licenses page (`/settings/licenses`). No React and no file access: the view hands in the
// text of `os/LICENSE.md` and `os/THIRD-PARTY-NOTICES.md` (imported at build time with `?raw`), these functions
// only describe it.

/** Lines as an editor counts them: a final newline does not start another line, and an empty text has none. */
export function countLines(text: string): number {
  if (text === "") return 0;
  const lines = text.split(/\r?\n/);
  if (lines[lines.length - 1] === "") lines.pop();
  return lines.length;
}

/** "1 line", "169 lines". */
export function linesText(n: number): string {
  return `${n} ${n === 1 ? "line" : "lines"}`;
}

/** Text of the first `# ` heading outside a fenced code block, or null. The view shows it above the document. */
export function firstHeading(text: string): string | null {
  let fence: string | null = null;
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trimStart();
    const mark = /^(`{3,}|~{3,})/.exec(line)?.[1];
    if (mark) {
      if (fence === null) fence = mark[0] as string;
      else if (mark[0] === fence) fence = null;
      continue;
    }
    if (fence !== null) continue;
    const h = /^#\s+(.+?)\s*#*\s*$/.exec(raw);
    if (h) return (h[1] as string).trim() || null;
  }
  return null;
}

/** "111network License · 169 lines": the document's own name (the page drops its leading `# Title`) and its size. */
export function describeDoc(text: string): string {
  return [firstHeading(text), linesText(countLines(text))].filter(Boolean).join(" · ");
}

/** Summary of the disclosure that holds the long notices file. */
export function noticesLabel(lines: number): string {
  return `Show all notices (${linesText(lines)})`;
}

/** What the page says while the notices chunk is loading, and when it could not be. */
export const NOTICES_LOADING = "Reading the notices…";
export const NOTICES_FAILED = "The notices could not be loaded. Reload the page and try again.";
