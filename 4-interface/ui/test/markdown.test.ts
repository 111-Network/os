// The KB markdown reader: safety (escaping, link policy), supported syntax, frontmatter, KB paths.
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  buildSlugIndex,
  classifyHref,
  frontmatterRows,
  inlineText,
  kbRoutePath,
  kbRouteTarget,
  MAX_DOC_CHARS,
  normalizeKbPath,
  parseInline,
  parseMarkdown,
  resolveRelative,
  splitFrontmatter,
  type Block,
  type Inline,
} from "../src/lib/markdown";
import { Blocks } from "../src/views/kb/render";

const html = (md: string, base = "projects/meshtastic.md"): string => renderToStaticMarkup(h(Blocks, { blocks: parseMarkdown(md, { basePath: base }).blocks }));
const blocks = (md: string): Block[] => parseMarkdown(md).blocks;
const text = (nodes: Inline[]): string => inlineText(nodes);

describe("escaping: no raw HTML ever reaches the page", () => {
  it("shows script tags as text", () => {
    const out = html("<script>alert(1)</script>\n\nhello <img src=x onerror=alert(1)> world");
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<img");
    expect(out).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt;");
  });

  it("escapes inside code spans and fenced code", () => {
    const out = html("`<b>x</b>`\n\n```html\n<iframe src=//evil></iframe>\n```");
    expect(out).not.toContain("<b>");
    expect(out).not.toContain("<iframe");
    expect(out).toContain("&lt;iframe src=//evil&gt;&lt;/iframe&gt;");
  });

  it("escapes link labels, quotes and ampersands", () => {
    const out = html('[a"b<c>&d](https://example.com/?q="x"&y=1)');
    expect(out).not.toContain("<c>");
    expect(out).toContain("a&quot;b&lt;c&gt;&amp;d");
  });

  it("a hostile document cannot add an attribute or a tag through any construct", () => {
    const evil = ["# <svg onload=1>", "- <a href=javascript:1>x</a>", "> <script>", "**<u>b</u>**", "[[<x>]]", "[<s>](a.md)"].join("\n");
    const out = html(evil);
    expect(out).not.toMatch(/<(script|svg|u|s|x|iframe|img)[\s>]/);
    expect(out).not.toMatch(/href="javascript/i);
  });

  it("comments are skipped, not injected", () => {
    const out = html("<!-- <script>alert(1)</script> -->\n\nvisible");
    expect(out).not.toContain("script");
    expect(out).toContain("visible");
  });
});

describe("link policy", () => {
  it("allows http and https, normalized, and nothing with credentials", () => {
    expect(classifyHref("https://github.com/automerge/automerge")).toEqual({ kind: "external", href: "https://github.com/automerge/automerge" });
    expect(classifyHref("http://111.local/x")).toEqual({ kind: "external", href: "http://111.local/x" });
    expect(classifyHref("https://user:pw@evil.example/")).toBeNull();
    expect(classifyHref("https://")).toBeNull();
  });

  it("refuses every other scheme, however it is dressed up", () => {
    for (const bad of [
      "javascript:alert(1)",
      "JaVaScRiPt:alert(1)",
      " javascript:alert(1)",
      "java\tscript:alert(1)",
      "java\nscript:alert(1)",
      "\u0001javascript:alert(1)",
      "data:text/html;base64,PHNjcmlwdD4=",
      "vbscript:x",
      "file:///etc/passwd",
      "mailto:a@b.c",
      "ftp://host/file",
      "tel:123",
      "blob:https://x/y",
      "about:blank",
      "https:\\\\evil.example",
    ]) {
      expect(classifyHref(bad), bad).toBeNull();
    }
  });

  it("refuses protocol-relative, absolute-path, anchor-only and non-markdown targets", () => {
    for (const bad of ["//evil.example/x.md", "/api/v1/settings", "/kb/projects/x.md", "#top", "image.png", "doc.pdf", "..\\x.md", "", "   "]) {
      expect(classifyHref(bad, "projects/a.md"), bad).toBeNull();
    }
  });

  it("whitespace inside an absolute URL is refused rather than repaired", () => {
    expect(classifyHref("https://exa mple.com")).toBeNull();
    expect(classifyHref("https://example.com/\nx")).toBeNull();
  });

  it("relative .md links resolve against the page they sit on", () => {
    expect(classifyHref("reticulum.md", "projects/meshtastic.md")).toEqual({ kind: "kb", path: "projects/reticulum.md" });
    expect(classifyHref("./reticulum.md#setup", "projects/meshtastic.md")).toEqual({ kind: "kb", path: "projects/reticulum.md" });
    expect(classifyHref("../devices/heltec.md", "projects/meshtastic.md")).toEqual({ kind: "kb", path: "devices/heltec.md" });
    expect(classifyHref("projects/x.md", "")).toEqual({ kind: "kb", path: "projects/x.md" });
    expect(classifyHref("a%20b.md", "projects/x.md")).toEqual({ kind: "kb", path: "projects/a b.md" });
  });

  it("relative links cannot climb out of the knowledge base", () => {
    expect(classifyHref("../../etc/passwd.md", "projects/meshtastic.md")).toBeNull();
    expect(classifyHref("../x.md", "top.md")).toBeNull();
    expect(classifyHref("%2e%2e/%2e%2e/x.md", "projects/a.md")).toBeNull();
    expect(classifyHref("%2e%2e/x.md", "projects/a.md")).toEqual({ kind: "kb", path: "x.md" }); // one level up from projects/ is still inside
    expect(classifyHref("a%2F..%2F..%2F..%2Fx.md", "projects/a.md")).toBeNull();
    expect(classifyHref("bad%zz.md", "projects/a.md")).toBeNull();
  });

  it("a refused link keeps its words and loses the anchor", () => {
    const out = html("see [the trick](javascript:alert(1)) now");
    expect(out).toContain("see the trick now");
    expect(out).not.toContain("<a");
  });

  it("external links open in a new tab without opener access; KB links stay in the app", () => {
    const out = html("[up](https://example.com) and [down](reticulum.md)");
    expect(out).toContain('href="https://example.com/"');
    expect(out).toContain('target="_blank"');
    expect(out).toContain('rel="noopener noreferrer"');
    expect(out).toContain('href="#/kb/projects/reticulum.md"');
    expect(out.match(/target="_blank"/g)).toHaveLength(1);
  });

  it("bare URLs become links, trailing punctuation stays outside", () => {
    const nodes = parseInline("see https://example.com/a, and (https://example.com/b).");
    const links = nodes.filter((n) => n.t === "link");
    expect(links).toHaveLength(2);
    expect(links[0]).toMatchObject({ link: { kind: "external", href: "https://example.com/a" } });
    expect(text(nodes)).toBe("see https://example.com/a, and (https://example.com/b).");
  });
});

describe("wikilinks", () => {
  it("link to <slug>.md by default", () => {
    const [n] = parseInline("[[meshtastic]]");
    expect(n).toMatchObject({ t: "link", link: { kind: "kb", path: "meshtastic.md" } });
    expect(text([n as Inline])).toBe("meshtastic");
  });

  it("support folder/slug and a label", () => {
    expect(parseInline("[[projects/reticulum|the stack]]")[0]).toMatchObject({ link: { path: "projects/reticulum.md" } });
    expect(text(parseInline("[[projects/reticulum|the stack]]"))).toBe("the stack");
  });

  it("use the index when one is supplied", () => {
    const idx = buildSlugIndex("- [Meshtastic](projects/meshtastic.md): x\n- [Heltec](devices/heltec-wifi-lora-32-v3.md): y\n- [Dup](devices/meshtastic.md): z");
    expect(idx.get("meshtastic")).toBe("projects/meshtastic.md");
    expect(idx.get("heltec-wifi-lora-32-v3")).toBe("devices/heltec-wifi-lora-32-v3.md");
    const [n] = parseInline("[[heltec-wifi-lora-32-v3]]", { resolveWiki: (s) => idx.get(s) ?? null });
    expect(n).toMatchObject({ link: { path: "devices/heltec-wifi-lora-32-v3.md" } });
  });

  it("an unusable slug stays as text", () => {
    expect(text(parseInline("[[../../etc/passwd]]"))).toBe("[[../../etc/passwd]]");
    expect(parseInline("[[../../etc/passwd]]").some((n) => n.t === "link")).toBe(false);
    expect(parseInline("[[ ]]")).toEqual([{ t: "text", text: "[[ ]]" }]);
  });
});

describe("blocks", () => {
  it("headings keep their level and drop the closing hashes", () => {
    const b = blocks("## What it is ##\n### Details\n#### Four\n#nospace");
    expect(b.map((x) => (x.t === "heading" ? x.level : x.t))).toEqual([2, 3, 4, "paragraph"]);
    expect(b[0]).toMatchObject({ t: "heading", children: [{ t: "text", text: "What it is" }] });
  });

  it("paragraph lines join, blank lines split", () => {
    const b = blocks("one\ntwo\n\nthree");
    expect(b).toHaveLength(2);
    expect(b[0]).toMatchObject({ t: "paragraph", children: [{ t: "text", text: "one two" }] });
  });

  it("bullet lists with all three markers, and numbered lists keeping their start", () => {
    const b = blocks("- a\n* b\n+ c\n\n3. x\n4) y");
    expect(b[0]).toMatchObject({ t: "list", ordered: false });
    expect((b[0] as Extract<Block, { t: "list" }>).items).toHaveLength(3);
    expect(b[1]).toMatchObject({ t: "list", ordered: true, start: 3 });
    expect((b[1] as Extract<Block, { t: "list" }>).items).toHaveLength(2);
  });

  it("nests lists by indent and joins wrapped item lines", () => {
    const [list] = blocks("- parent line one\n  continued here\n  - child a\n  - child b\n    - grandchild\n- second");
    const l = list as Extract<Block, { t: "list" }>;
    expect(l.items).toHaveLength(2);
    expect(text(l.items[0]?.children ?? [])).toBe("parent line one continued here");
    const kids = l.items[0]?.sub[0] as Extract<Block, { t: "list" }>;
    expect(kids.items).toHaveLength(2);
    expect((kids.items[1]?.sub[0] as Extract<Block, { t: "list" }>).items).toHaveLength(1);
  });

  it("a blank line inside a list does not end it; a paragraph does", () => {
    const b = blocks("- a\n\n- b\n\nafter");
    expect(b).toHaveLength(2);
    expect((b[0] as Extract<Block, { t: "list" }>).items).toHaveLength(2);
  });

  it("switching from bullets to numbers starts a new list", () => {
    const b = blocks("- a\n1. b");
    expect(b.map((x) => x.t)).toEqual(["list", "list"]);
  });

  it("fenced code keeps its text verbatim and is not parsed inside", () => {
    const [c] = blocks("```bash\nsudo -u node111 111 identity backup\n**not bold** [x](javascript:1)\n```");
    expect(c).toEqual({ t: "code", lang: "bash", text: "sudo -u node111 111 identity backup\n**not bold** [x](javascript:1)" });
  });

  it("tilde fences, longer fences and unterminated fences", () => {
    expect(blocks("~~~\na\n~~~")[0]).toMatchObject({ t: "code", text: "a" });
    expect(blocks("````\n```\ninner\n```\n````")[0]).toMatchObject({ t: "code", text: "```\ninner\n```" });
    expect(blocks("```\nnever closed\nstill code")[0]).toMatchObject({ t: "code", text: "never closed\nstill code" });
  });

  it("triple backticks inside a sentence are not a fence", () => {
    expect(blocks("use ```x``` here")[0]?.t).toBe("paragraph");
  });

  it("blockquotes and rules", () => {
    const b = blocks("> quoted\n> more\n\n---\n\n***");
    expect(b.map((x) => x.t)).toEqual(["quote", "rule", "rule"]);
    expect((b[0] as Extract<Block, { t: "quote" }>).children[0]).toMatchObject({ t: "paragraph" });
  });

  it("an ordered item only interrupts a paragraph when it starts at 1", () => {
    expect(blocks("In\n2026. A year").map((x) => x.t)).toEqual(["paragraph"]);
    expect(blocks("Steps\n1. first").map((x) => x.t)).toEqual(["paragraph", "list"]);
  });

  it("tables degrade to readable paragraph text", () => {
    const b = blocks("| a | b |\n|---|---|\n| 1 | 2 |");
    expect(b).toHaveLength(1);
    expect(b[0]?.t).toBe("paragraph");
  });

  it("html comments, one-line and multi-line, vanish; an unterminated one stays visible", () => {
    expect(blocks("<!-- generated -->\n\n## T")[0]?.t).toBe("heading");
    expect(blocks("<!-- a\nb\nc -->\ntext")).toHaveLength(1);
    expect(blocks("<!-- never closed\ntext")[0]?.t).toBe("paragraph");
  });
});

describe("inline", () => {
  it("code, bold and emphasis", () => {
    const n = parseInline("a `b *c*` **d** *e* f");
    expect(n.map((x) => x.t)).toEqual(["text", "code", "text", "strong", "text", "em", "text"]);
    expect(n[1]).toEqual({ t: "code", text: "b *c*" });
  });

  it("lone asterisks and snake_case are left alone", () => {
    expect(text(parseInline("2 * 3 * 4"))).toBe("2 * 3 * 4");
    expect(parseInline("2 * 3 * 4").every((n) => n.t === "text")).toBe(true);
    expect(text(parseInline("model_id and sha_256"))).toBe("model_id and sha_256");
    expect(text(parseInline("**unclosed"))).toBe("**unclosed");
    expect(text(parseInline("`unclosed"))).toBe("`unclosed");
  });

  it("backslash escapes", () => {
    expect(text(parseInline("\\*not em\\* and \\[x\\](y)"))).toBe("*not em* and [x](y)");
  });

  it("links take a title and angle brackets, and nest formatting in the label", () => {
    const [n] = parseInline('[**bold** label](https://example.com "title")');
    expect(n).toMatchObject({ t: "link", link: { href: "https://example.com/" } });
    expect((n as Extract<Inline, { t: "link" }>).children[0]).toMatchObject({ t: "strong" });
    expect(parseInline("[x](<https://example.com/a>)")[0]).toMatchObject({ link: { href: "https://example.com/a" } });
  });

  it("an unmatched bracket is text", () => {
    expect(text(parseInline("[not a link] (really)"))).toBe("[not a link] (really)");
    expect(parseInline("[a](b").every((n) => n.t === "text")).toBe(true);
  });
});

describe("frontmatter", () => {
  const entry = `---
title: Automerge
category: project
status: candidate
summary: "Local-first CRDT: merge history."
tags: [crdt, sync, "storage", offline]
platforms: [linux, macos]
upstream: https://github.com/automerge/automerge
sources:
  - https://github.com/automerge/automerge
  - https://automerge.org
---
Intro.

# Automerge

## What it is
Body.
`;

  it("splits flat keys, inline lists, block lists and quoted values", () => {
    const { frontmatter, body } = splitFrontmatter(entry);
    expect(body.startsWith("Intro.")).toBe(true);
    const map = new Map(frontmatter?.entries);
    expect(map.get("title")).toBe("Automerge");
    expect(map.get("summary")).toBe("Local-first CRDT: merge history.");
    expect(map.get("tags")).toEqual(["crdt", "sync", "storage", "offline"]);
    expect(map.get("sources")).toEqual(["https://github.com/automerge/automerge", "https://automerge.org"]);
  });

  it("rows: title, status, category, tags first; the rest advanced-only", () => {
    const rows = frontmatterRows(splitFrontmatter(entry).frontmatter!);
    expect(rows.slice(0, 4)).toEqual([
      { key: "title", value: "Automerge", advanced: false },
      { key: "status", value: "candidate", advanced: false },
      { key: "category", value: "project", advanced: false },
      { key: "tags", value: "crdt, sync, storage, offline", advanced: false },
    ]);
    expect(rows.slice(4).every((r) => r.advanced)).toBe(true);
    expect(rows.map((r) => r.key)).toContain("upstream");
  });

  it("is not frontmatter unless it opens the document and closes", () => {
    expect(splitFrontmatter("text\n---\ntitle: x\n---\n").frontmatter).toBeNull();
    expect(splitFrontmatter("---\ntitle: x\nno closing").frontmatter).toBeNull();
    expect(splitFrontmatter("---\n---\nbody").frontmatter).toEqual({ entries: [] });
    expect(splitFrontmatter("﻿---\ntitle: x\n---\nbody")).toMatchObject({ frontmatter: { entries: [["title", "x"]] }, body: "body" });
    expect(splitFrontmatter("---\r\ntitle: x\r\n---\r\nbody").body).toBe("body");
  });

  it("ignores what flat parsing cannot hold, and bounds what it keeps", () => {
    const fm = splitFrontmatter("---\ntitle: x\nnotes: |\n  multi\n  line\nlong: " + "y".repeat(2000) + "\n---\n").frontmatter!;
    const map = new Map(fm.entries);
    expect(map.has("notes")).toBe(false);
    expect((map.get("long") as string).length).toBe(500);
  });

  it("frontmatter values are data: a script in a value is just text", () => {
    const doc = parseMarkdown("---\ntitle: <script>alert(1)</script>\n---\nbody");
    expect(doc.title).toBe("<script>alert(1)</script>");
  });
});

describe("document", () => {
  it("title comes from frontmatter, else the first heading; a repeated leading # is dropped", () => {
    const withFm = parseMarkdown("---\ntitle: Automerge\n---\n# Automerge\n\ntext");
    expect(withFm.title).toBe("Automerge");
    expect(withFm.blocks.map((b) => b.t)).toEqual(["paragraph"]);

    const noFm = parseMarkdown("# Meshtastic\n\n## What\n\ntext");
    expect(noFm.title).toBe("Meshtastic");
    expect(noFm.blocks.map((b) => b.t)).toEqual(["heading", "paragraph"]);

    const differs = parseMarkdown("---\ntitle: Short\n---\n# A longer heading\ntext");
    expect(differs.title).toBe("Short");
    expect(differs.blocks[0]?.t).toBe("heading");

    expect(parseMarkdown("no title here").title).toBeNull();
  });

  it("renders a realistic entry end to end", () => {
    const md = "---\ntitle: Meshtastic\nstatus: testing\n---\n# Meshtastic\n\nIntro with `code` and [[reticulum]].\n\n## Set up\n1. Plug in.\n2. Run:\n\n```sh\n111 dashboard\n```\n";
    const out = html(md);
    expect(out).toContain("<h3");
    expect(out).toContain("<ol");
    expect(out).toContain("<code>111 dashboard</code>");
    expect(out).toContain('href="#/kb/reticulum.md"');
  });

  it("cuts a huge document instead of choking on it", () => {
    const big = "word ".repeat(MAX_DOC_CHARS / 4);
    const doc = parseMarkdown(big);
    expect(doc.truncated).toBe(true);
  });

  it("pathological input stays fast", () => {
    const t0 = Date.now();
    parseMarkdown("[".repeat(60_000));
    parseMarkdown("[a](".repeat(5000));
    parseMarkdown("*a ".repeat(10_000));
    parseMarkdown("`".repeat(30_000));
    parseMarkdown("- ".repeat(5000) + "x");
    parseMarkdown(Array.from({ length: 2000 }, (_, i) => `${" ".repeat(i % 12)}- item`).join("\n"));
    parseMarkdown("> ".repeat(500) + "deep");
    expect(Date.now() - t0).toBeLessThan(3000);
  });
});

describe("knowledge-base paths", () => {
  it("normalizeKbPath accepts plain relative paths only", () => {
    expect(normalizeKbPath("projects/meshtastic.md")).toBe("projects/meshtastic.md");
    for (const bad of ["", "/etc/passwd", "../x.md", "a/../b.md", "a//b.md", "./a.md", "a\\b.md", "a\u0000b.md", "a\nb.md", "x".repeat(600)]) {
      expect(normalizeKbPath(bad), bad).toBeNull();
    }
  });

  it("kbRouteTarget reads the route path", () => {
    expect(kbRouteTarget("/kb")).toEqual({ kind: "root" });
    expect(kbRouteTarget("/kb/projects/meshtastic.md")).toEqual({ kind: "entry", path: "projects/meshtastic.md" });
    expect(kbRouteTarget("/kb/projects/a%20b.md")).toEqual({ kind: "entry", path: "projects/a b.md" });
    for (const bad of ["/kb/..", "/kb/a/../b.md", "/kb/%2e%2e/x.md", "/kb/a%2Fb.md", "/kb/a%5Cb.md", "/kb/%zz", "/kb/a%00b.md", "/other"]) {
      expect(kbRouteTarget(bad), bad).toEqual({ kind: "invalid" });
    }
  });

  it("kbRoutePath and kbRouteTarget round-trip", () => {
    for (const p of ["projects/meshtastic.md", "devices/a b.md", "x/y/z.md", "weird#name?.md"]) {
      expect(kbRouteTarget(kbRoutePath(p))).toEqual({ kind: "entry", path: p });
    }
    expect(kbRoutePath("devices/a b.md")).toBe("/kb/devices/a%20b.md");
  });

  it("resolveRelative", () => {
    expect(resolveRelative("projects/a.md", "b.md")).toBe("projects/b.md");
    expect(resolveRelative("projects/a.md", "../devices/c.md")).toBe("devices/c.md");
    expect(resolveRelative("a.md", "b/c.md")).toBe("b/c.md");
    expect(resolveRelative("", "b.md")).toBe("b.md");
    expect(resolveRelative("a.md", "../b.md")).toBeNull();
  });
});
