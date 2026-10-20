// Static (offline) lint of a panel bundle directory — what the daemon serves unmodified (os/5-messaging/README.md
// section 13 "Offline lint"). No browser: it reads the files.
//
//   - every <script src>, <link href>, <img src|srcset>, <source>, <iframe>, <video|audio|track>, <object|embed>,
//     <form action>, <base href> and <meta http-equiv=refresh> in an .html file is relative or root-relative
//     (same-origin by construction) or a data: URI — nothing with a scheme or a host;
//   - no inline event-handler attribute (on*=), no javascript: URL;
//   - information: inline <script> blocks, and the third-party hostnames found by grep in every text file
//     (URL literals in the JS, CSS and JSON, not only the HTML) — which of them the manifest declares.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, extname } from "node:path";

const TEXT_EXT = new Set([".html", ".htm", ".js", ".mjs", ".cjs", ".css", ".json", ".webmanifest", ".svg", ".xml", ".txt", ".map"]);
const MAX_SCAN = 32 * 1024 * 1024;
// URL literals that are namespace identifiers, not requests
const NAMESPACE_HOSTS = [/(^|\.)w3\.org$/, /^schemas\.openxmlformats\.org$/, /^purl\.org$/, /^ns\.adobe\.com$/];

const RESOURCE_ATTRS = {
  script: ["src"],
  link: ["href"],
  img: ["src", "srcset"],
  source: ["src", "srcset"],
  iframe: ["src"],
  frame: ["src"],
  embed: ["src"],
  object: ["data"],
  video: ["src", "poster"],
  audio: ["src"],
  track: ["src"],
  input: ["src"],
  form: ["action"],
  base: ["href"],
  image: ["href", "xlink:href"],
  use: ["href", "xlink:href"],
};

function* walk(dir) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) yield* walk(p);
    else if (st.isFile()) yield [p, st];
  }
}

function attrsOf(s) {
  const out = [];
  const re = /([^\s=\/"'<>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?/g;
  let m;
  while ((m = re.exec(s))) out.push([m[1].toLowerCase(), m[2] ?? m[3] ?? m[4] ?? ""]);
  return out;
}

/** Classify one URL attribute value. */
export function classifyUrl(v) {
  const u = v.trim();
  if (u === "" || u.startsWith("#")) return "fragment";
  if (/^data:/i.test(u)) return "data";
  if (/^javascript:/i.test(u)) return "javascript";
  if (u.startsWith("//")) return "protocol-relative";
  if (/^[a-z][a-z0-9+.-]*:/i.test(u)) return "absolute";
  if (u.startsWith("/")) return "root-relative";
  return "relative";
}

const hostOfLiteral = (lit) => {
  try {
    return new URL(lit.startsWith("//") ? `http:${lit}` : lit).hostname.toLowerCase();
  } catch {
    return "";
  }
};

/** Lint `dir`; returns { results: [{id, level, text}], hosts: Map<host, {n, files:Set}> }. */
export function lintBundle(dir, { optionalHosts = [], entry = "index.html" } = {}) {
  const optional = new Set(optionalHosts.map((h) => h.toLowerCase()));
  const results = [];
  const add = (level, id, text) => results.push({ id, level, text });
  const external = [];
  const handlers = [];
  const jsUrls = [];
  let htmlFiles = 0;
  let inlineScripts = 0;
  let files = 0;
  let bytes = 0;
  const hosts = new Map();
  const classes = { relative: 0, "root-relative": 0, data: 0, fragment: 0 };

  let hasEntry = false;
  for (const [p, st] of walk(dir)) {
    files++;
    bytes += st.size;
    const rel = relative(dir, p);
    if (rel === entry) hasEntry = true;
    const ext = extname(p).toLowerCase();
    if (!TEXT_EXT.has(ext) || st.size > MAX_SCAN) continue;
    const text = readFileSync(p, "utf8");
    // third-party hostnames by grep, everywhere
    const re = /\b(?:https?|wss?):\/\/([a-z0-9](?:[a-z0-9.-]*[a-z0-9])?)(?::\d+)?/gi;
    let m;
    while ((m = re.exec(text))) {
      const h = m[1].toLowerCase();
      if (h === "localhost" || /^127\./.test(h) || NAMESPACE_HOSTS.some((r) => r.test(h))) continue;
      if (!hosts.has(h)) hosts.set(h, { n: 0, files: new Set() });
      const e = hosts.get(h);
      e.n++;
      e.files.add(rel);
    }
    if (ext !== ".html" && ext !== ".htm") continue;
    htmlFiles++;
    const html = text.replace(/<!--[\s\S]*?-->/g, "");
    const tagRe = /<([a-zA-Z][a-zA-Z0-9:-]*)\b([^>]*)>/g;
    let t;
    while ((t = tagRe.exec(html))) {
      const tag = t[1].toLowerCase();
      const attrs = attrsOf(t[2]);
      for (const [name, value] of attrs) {
        if (/^on[a-z]+$/.test(name)) handlers.push(`${rel}: <${tag} ${name}=…>`);
        // a javascript: URL is an inline handler by another name, on any tag (anchors included)
        if (/^(href|src|action|formaction|xlink:href|data|poster)$/.test(name) && /^\s*javascript:/i.test(value)) jsUrls.push(`${rel}: <${tag} ${name}>`);
      }
      if (tag === "script" && !attrs.some(([n]) => n === "src")) inlineScripts++;
      const want = RESOURCE_ATTRS[tag] || [];
      const check = (v, where) => {
        const k = classifyUrl(v);
        if (k === "javascript") return; // reported above, once
        if (k === "absolute" || k === "protocol-relative") {
          const h = hostOfLiteral(v.trim());
          external.push({ where: `${rel}: <${tag} ${where}=${v.trim().slice(0, 80)}>`, host: h, declared: optional.has(h) });
        } else classes[k] = (classes[k] || 0) + 1;
      };
      for (const [name, value] of attrs) {
        if (!want.includes(name)) continue;
        if (name === "srcset") for (const part of value.split(",")) check(part.trim().split(/\s+/)[0] || "", name);
        else check(value, name);
      }
      if (tag === "meta") {
        const he = attrs.find(([n]) => n === "http-equiv");
        const ct = attrs.find(([n]) => n === "content");
        if (he && he[1].toLowerCase() === "refresh" && ct) {
          const mm = /url\s*=\s*['"]?([^'";]+)/i.exec(ct[1]);
          if (mm) check(mm[1], "content(refresh)");
        }
      }
    }
  }

  add(hasEntry && htmlFiles > 0 ? "ok" : "FAIL", "bundle.entry", hasEntry ? `${files} files, ${bytes} bytes, ${htmlFiles} html; entry ${entry} present` : `no ${entry} in ${dir}`);
  const bad = external.filter((e) => !e.declared);
  const declared = external.filter((e) => e.declared);
  add(
    bad.length === 0 ? "ok" : "FAIL",
    "bundle.same-origin-srcs",
    bad.length === 0
      ? `every script/link/img/frame/form reference in the served HTML is relative (${classes.relative} relative, ${classes["root-relative"]} root-relative, ${classes.data} data:, no scheme or host)`
      : `${bad.length} HTML reference(s) leave the origin: ${bad.slice(0, 5).map((e) => e.where).join("; ")}${bad.length > 5 ? " …" : ""}`,
  );
  if (declared.length) add("note", "bundle.declared-srcs", `${declared.length} HTML reference(s) to declared optional hosts: ${declared.map((e) => e.where).join("; ")}`);
  add(handlers.length === 0 ? "ok" : "FAIL", "bundle.inline-handlers", handlers.length === 0 ? "no inline event-handler attribute (on*=) in any HTML file" : `inline handlers: ${handlers.slice(0, 6).join("; ")}`);
  add(jsUrls.length === 0 ? "ok" : "FAIL", "bundle.javascript-urls", jsUrls.length === 0 ? "no javascript: URL in any HTML file" : `javascript: URLs: ${jsUrls.join("; ")}`);
  add("note", "bundle.inline-scripts", `${inlineScripts} inline <script> block(s) (the panel origin sets no script CSP of its own)`);
  const sorted = [...hosts.entries()].sort((a, b) => b[1].n - a[1].n || a[0].localeCompare(b[0]));
  if (sorted.length === 0) add("note", "bundle.third-party-hosts", "no third-party hostname literal found by grep in any text file");
  for (const [h, e] of sorted) {
    add(
      "note",
      "bundle.third-party-host",
      `${h} x${e.n} in ${[...e.files].slice(0, 3).join(", ")}${e.files.size > 3 ? ", …" : ""} — ${optional.has(h) ? "declared in [offline] optional_hosts" : "NOT declared (information; the runtime recording decides)"}`,
    );
  }
  return { results, hosts };
}
