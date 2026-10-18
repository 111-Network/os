// A tiny, safe markdown reader for the knowledge-base viewer (spec 6.x, task U4).
//
// Safety model: this module never produces HTML. It parses text into a small AST of plain strings;
// the React renderer (views/kb/render.tsx) turns the AST into elements, so every character reaches
// the page as an escaped text node. There is no node type for raw HTML, so `<script>` or `<img onerror>`
// in a document is displayed as the literal text it is. Links pass a strict allowlist (`classifyHref`).
//
// Supported: `#`..`######` headings, paragraphs, `-`/`*`/`+` and `1.` lists (nested by indent), fenced
// code, blockquotes, rules, inline code, **bold**, *emphasis*, [text](url), [[wikilinks]], bare http(s)
// URLs, and a YAML frontmatter block (flat `key: value`, `[a, b]` and `- item` lists). Not supported:
// tables, images, footnotes, setext headings, reference links. They degrade to readable plain text.

/** Hard caps so a hostile or broken document cannot stall the tab. */
export const MAX_DOC_CHARS = 512_000;
export const MAX_INLINE_CHARS = 20_000;
const MAX_LIST_DEPTH = 4;
const MAX_QUOTE_DEPTH = 3;
const MAX_INLINE_DEPTH = 4;

// ------------------------------------------------------------------------------------------ AST

export type Inline =
  | { t: "text"; text: string }
  | { t: "code"; text: string }
  | { t: "strong"; children: Inline[] }
  | { t: "em"; children: Inline[] }
  | { t: "link"; link: LinkTarget; children: Inline[] };

export type LinkTarget = { kind: "external"; href: string } | { kind: "kb"; path: string };

export interface ListItem {
  children: Inline[];
  /** Nested lists under this item. */
  sub: Block[];
}

export type Block =
  | { t: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; children: Inline[] }
  | { t: "paragraph"; children: Inline[] }
  | { t: "list"; ordered: boolean; start: number; items: ListItem[] }
  | { t: "code"; lang: string; text: string }
  | { t: "quote"; children: Block[] }
  | { t: "rule" };

export interface Frontmatter {
  /** In document order. A value is a string or a list of strings. */
  entries: Array<[key: string, value: string | string[]]>;
}

export interface MdDoc {
  frontmatter: Frontmatter | null;
  /** Frontmatter `title`, else the first `#` heading. */
  title: string | null;
  blocks: Block[];
  /** True when the document was longer than MAX_DOC_CHARS and was cut. */
  truncated: boolean;
}

export interface MdOptions {
  /** KB-relative path of the document being parsed (`projects/meshtastic.md`); '' for the index. */
  basePath?: string;
  /** Maps `[[slug]]` to a KB path. Return null to fall back to `<slug>.md`. */
  resolveWiki?: (slug: string) => string | null;
}

// ------------------------------------------------------------------------------- KB paths / links

const CONTROL = /[\u0000-\u001f\u007f]/;

/**
 * A KB-relative path the viewer will request: no leading slash, no `.`/`..`/empty segments, no
 * backslashes or control characters. The daemon guards traversal too; this keeps bad paths off the wire.
 */
export function normalizeKbPath(raw: string): string | null {
  if (raw === "" || raw.length > 512) return null;
  if (raw.startsWith("/") || raw.includes("\\") || CONTROL.test(raw)) return null;
  const segs = raw.split("/");
  if (segs.some((s) => s === "" || s === "." || s === "..")) return null;
  return segs.join("/");
}

/** Route path for a KB entry: segments are percent-encoded, slashes kept. */
export function kbRoutePath(path: string): string {
  return `/kb/${path.split("/").map(encodeURIComponent).join("/")}`;
}

export type KbRoute = { kind: "root" } | { kind: "entry"; path: string } | { kind: "invalid" };

/** `/kb` -> root; `/kb/projects/a%20b.md` -> entry `projects/a b.md`; anything unsafe -> invalid. */
export function kbRouteTarget(routePath: string): KbRoute {
  if (routePath === "/kb") return { kind: "root" };
  if (!routePath.startsWith("/kb/")) return { kind: "invalid" };
  const decoded: string[] = [];
  for (const raw of routePath.slice("/kb/".length).split("/")) {
    let seg: string;
    try {
      seg = decodeURIComponent(raw);
    } catch {
      return { kind: "invalid" };
    }
    if (seg.includes("/") || seg.includes("\\")) return { kind: "invalid" };
    decoded.push(seg);
  }
  const path = normalizeKbPath(decoded.join("/"));
  return path ? { kind: "entry", path } : { kind: "invalid" };
}

/** Resolve a relative link against the KB path of the page it sits on. null = outside the KB root. */
export function resolveRelative(basePath: string, href: string): string | null {
  const dir = basePath.includes("/") ? basePath.slice(0, basePath.lastIndexOf("/")).split("/") : [];
  const out = [...dir];
  for (const seg of href.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      if (out.length === 0) return null;
      out.pop();
    } else out.push(seg);
  }
  return normalizeKbPath(out.join("/"));
}

/**
 * The link allowlist. http(s) opens in a new tab; a relative `.md` path becomes a KB route. Everything
 * else (javascript:, data:, mailto:, file:, protocol-relative, absolute paths, bare #anchors, other
 * file types) returns null and the label is shown as plain text.
 */
export function classifyHref(href: string, basePath = ""): LinkTarget | null {
  const raw = href.trim();
  if (raw === "" || raw.length > 2000) return null;
  // Browsers drop tabs and newlines inside a URL and ignore leading control characters: judge what they would parse.
  const squashed = raw.replace(/[\u0000- \u007f]/g, "");
  if (/^https?:\/\//i.test(squashed)) {
    if (raw !== squashed) return null; // whitespace or control characters inside an absolute URL: refuse
    try {
      const u = new URL(raw);
      if (u.username || u.password) return null; // https://trusted.example@evil.example looks like one site and is another
      return u.protocol === "http:" || u.protocol === "https:" ? { kind: "external", href: u.href } : null;
    } catch {
      return null;
    }
  }
  // Relative from here on. A colon anywhere means a scheme (or something that parses like one): refuse.
  if (squashed !== raw || raw.includes(":") || raw.includes("\\") || raw.startsWith("/") || raw.startsWith("#")) return null;
  const path = raw.replace(/[?#].*$/, "");
  if (!/\.md$/i.test(path)) return null;
  let decoded: string;
  try {
    decoded = path.split("/").map(decodeURIComponent).join("/");
  } catch {
    return null;
  }
  const resolved = resolveRelative(basePath, decoded);
  return resolved ? { kind: "kb", path: resolved } : null;
}

/** `[[slug]]`, `[[folder/slug]]`, `[[slug|label]]` -> KB path. Best effort: the index decides, else `<slug>.md`. */
export function wikiTarget(slug: string, opts: MdOptions = {}): string | null {
  const clean = slug.trim().replace(/\.md$/i, "");
  if (!/^[A-Za-z0-9][A-Za-z0-9._\-/]*$/.test(clean)) return null;
  const mapped = opts.resolveWiki?.(clean) ?? null;
  return normalizeKbPath(mapped ?? `${clean}.md`);
}

/**
 * slug -> KB path from the root `llms.txt`, whose lines read `- [Title](projects/meshtastic.md): ...`.
 * The slug is the file name without `.md`. The first entry wins when two folders share a slug.
 */
export function buildSlugIndex(indexText: string): Map<string, string> {
  const map = new Map<string, string>();
  const re = /\]\(([^)\s]+\.md)\)/g;
  for (const m of indexText.slice(0, MAX_DOC_CHARS).matchAll(re)) {
    const path = normalizeKbPath(m[1] ?? "");
    if (!path) continue;
    const slug = path.slice(path.lastIndexOf("/") + 1).replace(/\.md$/i, "");
    if (!map.has(slug)) map.set(slug, path);
    if (!map.has(path.replace(/\.md$/i, ""))) map.set(path.replace(/\.md$/i, ""), path);
  }
  return map;
}

// ------------------------------------------------------------------------------------ frontmatter

const unquote = (v: string): string => {
  const t = v.trim();
  return t.length >= 2 && (t[0] === '"' || t[0] === "'") && t[t.length - 1] === t[0] ? t.slice(1, -1) : t;
};

const FM_RE = /^---[ \t]*\r?\n(?:([\s\S]*?)\r?\n)?---[ \t]*(?:\r?\n|$)/;
const MAX_FM_ENTRIES = 40;
const MAX_FM_VALUE = 500;

export function splitFrontmatter(src: string): { frontmatter: Frontmatter | null; body: string } {
  const text = src.charCodeAt(0) === 0xfeff ? src.slice(1) : src;
  const m = FM_RE.exec(text);
  if (!m) return { frontmatter: null, body: text };
  const entries: Frontmatter["entries"] = [];
  let list: { key: string; items: string[] } | null = null;
  const flush = () => {
    if (list && list.items.length > 0 && entries.length < MAX_FM_ENTRIES) entries.push([list.key, list.items]);
    list = null;
  };
  for (const line of (m[1] ?? "").split(/\r?\n/)) {
    if (/^\s*(#.*)?$/.test(line)) continue;
    const item = /^\s+-\s+(.*)$/.exec(line);
    if (item) {
      if (list) list.items.push(unquote(item[1] ?? "").slice(0, MAX_FM_VALUE));
      continue;
    }
    const kv = /^([A-Za-z_][\w-]*)\s*:\s*(.*)$/.exec(line);
    if (!kv) continue; // indented continuation, multi-line scalars and anything else flat parsing cannot hold
    flush();
    const key = kv[1] as string;
    const value = (kv[2] ?? "").trim();
    if (value === "") list = { key, items: [] };
    else if (/^[|>][+-]?$/.test(value)) continue;
    else if (value.startsWith("[") && value.endsWith("]")) {
      const items = value.slice(1, -1).split(",").map((x) => unquote(x).slice(0, MAX_FM_VALUE)).filter((x) => x !== "");
      if (entries.length < MAX_FM_ENTRIES) entries.push([key, items]);
    } else if (entries.length < MAX_FM_ENTRIES) entries.push([key, unquote(value).slice(0, MAX_FM_VALUE)]);
  }
  flush();
  return { frontmatter: { entries }, body: text.slice(m[0].length) };
}

/** Rows for the spec table: the four keys readers care about first, the rest advanced-only. */
export const FRONTMATTER_PRIMARY = ["title", "status", "category", "tags"] as const;

export function frontmatterRows(fm: Frontmatter): Array<{ key: string; value: string; advanced: boolean }> {
  const text = (v: string | string[]) => (Array.isArray(v) ? v.join(", ") : v);
  const byKey = new Map(fm.entries);
  const primary = FRONTMATTER_PRIMARY.flatMap((k) => {
    const v = byKey.get(k);
    return v === undefined || text(v) === "" ? [] : [{ key: k, value: text(v), advanced: false }];
  });
  const rest = fm.entries
    .filter(([k, v]) => !(FRONTMATTER_PRIMARY as readonly string[]).includes(k) && text(v) !== "")
    .map(([k, v]) => ({ key: k, value: text(v), advanced: true }));
  return [...primary, ...rest];
}

// ----------------------------------------------------------------------------------------- inline

function bracketEnd(text: string, open: number, limit: number): number {
  let depth = 0;
  for (let i = open; i < text.length && i < open + limit; i++) {
    const c = text[i];
    if (c === "\\") i++;
    else if (c === "[") depth++;
    else if (c === "]" && --depth === 0) return i;
  }
  return -1;
}

function parenEnd(text: string, open: number, limit: number): number {
  let depth = 0;
  for (let i = open; i < text.length && i < open + limit; i++) {
    const c = text[i];
    if (c === "\\") i++;
    else if (c === "(") depth++;
    else if (c === ")" && --depth === 0) return i;
  }
  return -1;
}

const BARE_URL = /https?:\/\/[^\s<>()[\]"'`]+/y;

export function inlineText(nodes: readonly Inline[]): string {
  return nodes.map((n) => (n.t === "text" || n.t === "code" ? n.text : inlineText(n.children))).join("");
}

export function parseInline(input: string, opts: MdOptions = {}, depth = 0): Inline[] {
  const text = input.length > MAX_INLINE_CHARS ? input.slice(0, MAX_INLINE_CHARS) : input;
  const out: Inline[] = [];
  let buf = "";
  const flush = () => {
    if (buf) out.push({ t: "text", text: buf });
    buf = "";
  };
  const base = opts.basePath ?? "";
  let i = 0;
  while (i < text.length) {
    const c = text[i] as string;
    const next = text[i + 1];

    if (c === "\\" && next !== undefined && /[\\`*_{}[\]()#+\-.!|<>~]/.test(next)) {
      buf += next;
      i += 2;
      continue;
    }

    if (c === "`") {
      let n = 1;
      while (text[i + n] === "`") n++;
      const fence = "`".repeat(n);
      let close = text.indexOf(fence, i + n);
      while (close >= 0 && text[close + n] === "`") close = text.indexOf(fence, close + n + 1);
      if (close > 0) {
        flush();
        let code = text.slice(i + n, close);
        if (code.length > 2 && code.startsWith(" ") && code.endsWith(" ")) code = code.slice(1, -1);
        out.push({ t: "code", text: code });
        i = close + n;
        continue;
      }
      buf += fence;
      i += n;
      continue;
    }

    if (c === "*" && next === "*" && depth < MAX_INLINE_DEPTH) {
      const close = text.indexOf("**", i + 2);
      if (close > i + 2) {
        flush();
        out.push({ t: "strong", children: parseInline(text.slice(i + 2, close), opts, depth + 1) });
        i = close + 2;
        continue;
      }
    }

    if (c === "*" && next !== undefined && !/\s/.test(next) && next !== "*" && depth < MAX_INLINE_DEPTH) {
      let close = text.indexOf("*", i + 1);
      while (close > 0 && (/\s/.test(text[close - 1] ?? " ") || text[close + 1] === "*" || text[close - 1] === "*")) close = text.indexOf("*", close + 1);
      if (close > i + 1) {
        flush();
        out.push({ t: "em", children: parseInline(text.slice(i + 1, close), opts, depth + 1) });
        i = close + 1;
        continue;
      }
    }

    if (c === "[" && next === "[") {
      const close = text.indexOf("]]", i + 2);
      if (close > i + 2 && close - i < 200) {
        const inner = text.slice(i + 2, close);
        const bar = inner.indexOf("|");
        const slug = bar >= 0 ? inner.slice(0, bar) : inner;
        const label = (bar >= 0 ? inner.slice(bar + 1) : inner).trim() || slug.trim();
        const path = wikiTarget(slug, opts);
        flush();
        out.push(path ? { t: "link", link: { kind: "kb", path }, children: [{ t: "text", text: label }] } : { t: "text", text: `[[${inner}]]` });
        i = close + 2;
        continue;
      }
    }

    if (c === "[" && depth < MAX_INLINE_DEPTH) {
      const labelEnd = bracketEnd(text, i, 500);
      if (labelEnd > 0 && text[labelEnd + 1] === "(") {
        const urlEnd = parenEnd(text, labelEnd + 1, 2000);
        if (urlEnd > 0) {
          const label = text.slice(i + 1, labelEnd);
          const href = text.slice(labelEnd + 2, urlEnd).replace(/\s+("[^"]*"|'[^']*')\s*$/, "").replace(/^<(.*)>$/, "$1");
          const link = classifyHref(href, base);
          flush();
          const kids = parseInline(label, opts, depth + 1);
          if (link) out.push({ t: "link", link, children: kids });
          else out.push(...kids); // refused target: keep the words, drop the link
          i = urlEnd + 1;
          continue;
        }
      }
    }

    if (c === "h" && (i === 0 || !/[A-Za-z0-9]/.test(text[i - 1] as string))) {
      BARE_URL.lastIndex = i;
      const m = BARE_URL.exec(text);
      if (m) {
        const raw = m[0].replace(/[.,;:!?]+$/, "");
        const link = classifyHref(raw);
        if (link) {
          flush();
          out.push({ t: "link", link, children: [{ t: "text", text: raw }] });
          i += raw.length;
          continue;
        }
      }
    }

    buf += c;
    i++;
  }
  flush();
  return out;
}

// ------------------------------------------------------------------------------------------ blocks

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/;
const HEADING = /^ {0,3}(#{1,6})\s+(.*?)\s*$/;
const RULE = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/;
const QUOTE = /^ {0,3}>/;
const LIST_ITEM = /^( *)([-*+]|\d{1,9}[.)])\s+(.*)$/;

const expandTabs = (line: string): string => line.replace(/^[ \t]+/, (ws) => ws.replace(/\t/g, "    "));

interface FlatItem {
  indent: number;
  ordered: boolean;
  start: number;
  text: string;
}

/** Does `line` open a new block? An ordered item interrupts a paragraph only when it starts at 1 ("2026. A year" does not). */
function startsBlock(line: string): boolean {
  if (FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line)) return true;
  const item = LIST_ITEM.exec(line);
  return !!item && (!/\d/.test(item[2] as string) || Number.parseInt(item[2] as string, 10) === 1);
}

function buildList(items: FlatItem[], pos: { i: number }, depth: number, opts: MdOptions): Block {
  const first = items[pos.i] as FlatItem;
  const base = first.indent;
  const listItems: ListItem[] = [];
  while (pos.i < items.length) {
    const it = items[pos.i] as FlatItem;
    if (it.indent < base) break;
    const last = listItems[listItems.length - 1];
    if (it.indent > base && last) {
      if (depth < MAX_LIST_DEPTH) {
        last.sub.push(buildList(items, pos, depth + 1, opts));
        continue;
      }
    } else if (listItems.length > 0 && it.ordered !== first.ordered) break; // bullets after numbers: a new list
    listItems.push({ children: parseInline(it.text, opts), sub: [] });
    pos.i++;
  }
  return { t: "list", ordered: first.ordered, start: first.start, items: listItems };
}

function parseBlocks(lines: readonly string[], opts: MdOptions, quoteDepth: number): Block[] {
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = expandTabs(lines[i] as string);
    if (/^\s*$/.test(line)) {
      i++;
      continue;
    }

    const fence = FENCE.exec(line);
    if (fence) {
      const marker = fence[1] as string;
      const closing = new RegExp(`^ {0,3}${marker[0] === "`" ? "`" : "~"}{${marker.length},}\\s*$`);
      const buf: string[] = [];
      i++;
      while (i < lines.length && !closing.test(lines[i] as string)) buf.push(lines[i++] as string);
      i++; // the closing fence (or the end of an unterminated block)
      blocks.push({ t: "code", lang: (fence[2] ?? "").slice(0, 32), text: buf.join("\n") });
      continue;
    }

    // HTML comments (the generated index opens with one) are invisible in rendered markdown: skip whole-line comments.
    if (/^\s*<!--/.test(line)) {
      let end = -1;
      for (let j = i; j < lines.length && j < i + 200; j++) {
        if (/-->\s*$/.test(lines[j] as string)) {
          end = j;
          break;
        }
        if (j > i && /-->/.test(lines[j] as string)) break; // text follows the closing marker: not a pure comment
      }
      if (end >= 0 && (end > i || /^\s*<!--[\s\S]*-->\s*$/.test(line))) {
        i = end + 1;
        continue;
      }
    }

    const heading = HEADING.exec(line);
    if (heading) {
      blocks.push({ t: "heading", level: (heading[1] as string).length as 1 | 2 | 3 | 4 | 5 | 6, children: parseInline((heading[2] as string).replace(/\s+#+$/, ""), opts) });
      i++;
      continue;
    }

    if (RULE.test(line)) {
      blocks.push({ t: "rule" });
      i++;
      continue;
    }

    if (QUOTE.test(line)) {
      const inner: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i] as string)) inner.push((lines[i++] as string).replace(/^ {0,3}> ?/, ""));
      blocks.push({ t: "quote", children: quoteDepth < MAX_QUOTE_DEPTH ? parseBlocks(inner, opts, quoteDepth + 1) : [{ t: "paragraph", children: parseInline(inner.join(" "), opts) }] });
      continue;
    }

    if (LIST_ITEM.test(line)) {
      const flat: FlatItem[] = [];
      while (i < lines.length) {
        const cur = expandTabs(lines[i] as string);
        const m = LIST_ITEM.exec(cur);
        if (m && !RULE.test(cur)) {
          const marker = m[2] as string;
          const ordered = /\d/.test(marker);
          flat.push({ indent: (m[1] as string).length, ordered, start: ordered ? Number.parseInt(marker, 10) : 1, text: (m[3] as string).trim() });
          i++;
        } else if (/^\s*$/.test(cur)) {
          // A blank line continues the list only when an indented item follows.
          let j = i + 1;
          while (j < lines.length && /^\s*$/.test(lines[j] as string)) j++;
          if (j >= lines.length || !LIST_ITEM.test(expandTabs(lines[j] as string))) break;
          i = j;
        } else {
          const prev = flat[flat.length - 1] as FlatItem;
          const indent = (/^ */.exec(cur) as RegExpExecArray)[0].length;
          if (indent > prev.indent && !startsBlock(cur)) {
            prev.text += ` ${cur.trim()}`; // soft-wrapped continuation of the item
            i++;
          } else break;
        }
      }
      const pos = { i: 0 };
      while (pos.i < flat.length) blocks.push(buildList(flat, pos, 0, opts));
      continue;
    }

    const para: string[] = [];
    while (i < lines.length && !/^\s*$/.test(lines[i] as string) && (para.length === 0 || !startsBlock(expandTabs(lines[i] as string)))) {
      para.push((lines[i++] as string).trim());
    }
    blocks.push({ t: "paragraph", children: parseInline(para.join(" "), opts) });
  }
  return blocks;
}

export function parseMarkdown(src: string, opts: MdOptions = {}): MdDoc {
  const truncated = src.length > MAX_DOC_CHARS;
  const { frontmatter, body } = splitFrontmatter(truncated ? src.slice(0, MAX_DOC_CHARS) : src);
  let blocks = parseBlocks(body.split(/\r?\n/), opts, 0);
  const fmTitle = frontmatter?.entries.find(([k]) => k === "title")?.[1];
  const first = blocks[0];
  const h1 = first && first.t === "heading" && first.level === 1 ? inlineText(first.children).trim() : null;
  const title = (typeof fmTitle === "string" && fmTitle.trim() !== "" ? fmTitle.trim() : null) ?? (h1 || null);
  // The page header already shows the title: drop the document's own leading `# Title` when it repeats it.
  if (h1 !== null && title !== null && h1.toLowerCase() === title.toLowerCase()) blocks = blocks.slice(1);
  return { frontmatter, title, blocks, truncated };
}
