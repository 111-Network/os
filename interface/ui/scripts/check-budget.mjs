#!/usr/bin/env node
// Size budget gate (spec section 2): gzip every .js and .css under dist/ and fail above 200 KB combined.
// Buckets: core = chunks that contain react-dom code, app = every other .js, css = every .css.
// Fonts are listed separately and never counted. 1 KB = 1000 bytes (same unit Vite prints).
import { readdirSync, readFileSync, statSync, existsSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const LIMIT_KB = 200;
const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const argValue = (flag) => {
  const i = args.indexOf(flag);
  return i >= 0 ? args[i + 1] : undefined;
};
const dist = resolve(argValue("--dist") ?? join(here, "..", "dist"));
const limitBytes = Number(argValue("--limit-kb") ?? LIMIT_KB) * 1000;

// Strings that only react-dom's production build contains.
const REACT_DOM_MARKERS = ["__reactFiber$", "__reactContainer$", "_reactListening"];

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

const kb = (n) => (n / 1000).toFixed(1).padStart(7);
const pad = (s, n) => (s.length >= n ? s : s + " ".repeat(n - s.length));

if (!existsSync(dist) || !statSync(dist).isDirectory()) {
  console.error(`check-budget: ${dist} does not exist. Run "pnpm build" first.`);
  process.exit(1);
}

const files = walk(dist).sort();
const assets = [];
const fonts = [];
for (const file of files) {
  const rel = relative(dist, file);
  if (/\.(js|css)$/.test(file)) {
    const buf = readFileSync(file);
    const text = file.endsWith(".js") ? buf.toString("utf8") : "";
    const bucket = file.endsWith(".css") ? "css" : REACT_DOM_MARKERS.some((m) => text.includes(m)) ? "core" : "app";
    assets.push({ rel, bucket, raw: buf.length, gz: gzipSync(buf, { level: 9 }).length });
  } else if (/\.woff2?$/.test(file)) {
    fonts.push({ rel, raw: statSync(file).size });
  }
}

if (assets.length === 0) {
  console.error(`check-budget: no .js or .css files under ${dist}. Run "pnpm build" first.`);
  process.exit(1);
}

const preloaded = (() => {
  const index = join(dist, "index.html");
  if (!existsSync(index)) return [];
  const html = readFileSync(index, "utf8");
  return [...html.matchAll(/<link[^>]*rel="preload"[^>]*>/g)].map((m) => m[0]);
})();

const W = Math.max(24, ...assets.map((a) => a.rel.length), ...fonts.map((f) => f.rel.length));
console.log(`\nBundle budget  (dist: ${dist})\n`);
console.log(`${pad("file", W)}  ${pad("bucket", 6)}  ${"raw KB".padStart(8)}  ${"gzip KB".padStart(8)}`);
console.log(`${"-".repeat(W)}  ${"-".repeat(6)}  ${"-".repeat(8)}  ${"-".repeat(8)}`);
for (const a of assets) console.log(`${pad(a.rel, W)}  ${pad(a.bucket, 6)}  ${kb(a.raw)}  ${kb(a.gz)}`);

const buckets = { core: 0, app: 0, css: 0 };
for (const a of assets) buckets[a.bucket] += a.gz;
const total = buckets.core + buckets.app + buckets.css;

console.log(`\n${pad("bucket", 10)}  ${"gzip KB".padStart(8)}`);
console.log(`${"-".repeat(10)}  ${"-".repeat(8)}`);
for (const [name, bytes] of Object.entries(buckets)) console.log(`${pad(name, 10)}  ${kb(bytes)}`);
console.log(`${pad("TOTAL", 10)}  ${kb(total)}   limit ${kb(limitBytes).trim()} KB JS+CSS gzip`);

console.log(`\nFonts (listed only, not counted; woff2 is already compressed)\n`);
for (const f of fonts) {
  const isPreloaded = preloaded.some((l) => l.includes(f.rel.split("/").pop() ?? "\0"));
  console.log(`${pad(f.rel, W)}  ${kb(f.raw)} KB  ${isPreloaded ? "preloaded" : ""}`);
}
if (fonts.length === 0) console.log("(none found in dist)");

if (total > limitBytes) {
  console.error(`\nFAIL: JS+CSS gzip total ${kb(total).trim()} KB exceeds the ${kb(limitBytes).trim()} KB budget.`);
  process.exit(1);
}
console.log(`\nPASS: JS+CSS gzip total ${kb(total).trim()} KB is within the ${kb(limitBytes).trim()} KB budget.`);
