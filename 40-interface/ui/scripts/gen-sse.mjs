#!/usr/bin/env node
// gen-sse.mjs — second half of `pnpm gen:types` (the first half is openapi-typescript).
//
// Reads ../api/openapi.yaml with a tiny YAML-subset reader (no dependencies) and writes:
//   src/lib/api/sse.gen.ts    typed SSE frame unions from `x-sse-events`, convenience aliases,
//                             runtime enum tuples. Imports `components` from ./types.gen.
//   test/contract.gen.ts      a JSON digest of components.schemas + every operation, used only by
//                             vitest to validate fixtures and the HTTP adapter against the contract.
//
// Usage:  node scripts/gen-sse.mjs           write the files (only when content changed)
//         node scripts/gen-sse.mjs --check   exit 1 if the files on disk are stale
//
// The reader supports exactly what openapi.yaml uses: block maps, block sequences, flow maps and
// sequences on one line, quoted and plain scalars, `#` comments, `|`/`>` block scalars.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------------------------
// YAML subset
// ---------------------------------------------------------------------------------------------

function fail(msg) {
  throw new Error(`gen-sse: ${msg}`);
}

function isQuoteStart(s, k) {
  // A quote opens a quoted scalar only at a token start; an apostrophe inside prose does not.
  if (s[k] !== '"' && s[k] !== "'") return false;
  if (k === 0) return true;
  return /[\s\[{,:]/.test(s[k - 1]);
}

function closingQuote(s, start) {
  const q = s[start];
  for (let k = start + 1; k < s.length; k++) {
    if (q === '"' && s[k] === "\\") k++;
    else if (q === "'" && s[k] === "'" && s[k + 1] === "'") k++;
    else if (s[k] === q) return k;
  }
  return -1;
}

function stripComment(s) {
  for (let k = 0; k < s.length; k++) {
    if (isQuoteStart(s, k)) {
      const end = closingQuote(s, k);
      if (end < 0) return s;
      k = end;
    } else if (s[k] === "#" && (k === 0 || /\s/.test(s[k - 1]))) {
      return s.slice(0, k);
    }
  }
  return s;
}

function unquote(tok) {
  if (tok[0] === '"') {
    try {
      return JSON.parse(tok);
    } catch {
      return tok.slice(1, -1);
    }
  }
  return tok.slice(1, -1).replace(/''/g, "'");
}

function scalar(str) {
  const t = str.trim();
  if (t === "" || t === "null" || t === "~") return null;
  if (t === "true") return true;
  if (t === "false") return false;
  if (/^-?\d+$/.test(t)) return Number(t);
  if (/^-?\d+\.\d+$/.test(t)) return Number(t);
  return t;
}

function parseFlow(s) {
  let p = 0;
  const ws = () => {
    while (p < s.length && /\s/.test(s[p])) p++;
  };
  const quoted = () => {
    const end = closingQuote(s, p);
    if (end < 0) fail(`unterminated quote in: ${s}`);
    const v = unquote(s.slice(p, end + 1));
    p = end + 1;
    return v;
  };
  const plain = () => {
    const q = p;
    while (p < s.length && s[p] !== "," && s[p] !== "}" && s[p] !== "]") p++;
    return scalar(s.slice(q, p));
  };
  const key = () => {
    ws();
    if (s[p] === '"' || s[p] === "'") return quoted();
    const q = p;
    while (p < s.length && !(s[p] === ":" && (p + 1 >= s.length || /[\s,}\]]/.test(s[p + 1])))) p++;
    return s.slice(q, p).trim();
  };
  const value = () => {
    ws();
    if (s[p] === "{") return map();
    if (s[p] === "[") return seq();
    if (s[p] === '"' || s[p] === "'") return quoted();
    return plain();
  };
  const map = () => {
    p++;
    const o = {};
    for (;;) {
      ws();
      if (s[p] === "}") {
        p++;
        return o;
      }
      const k = key();
      ws();
      if (s[p] !== ":") fail(`expected ':' in flow map: ${s}`);
      p++;
      ws();
      o[k] = s[p] === "," || s[p] === "}" ? null : value();
      ws();
      if (s[p] === ",") p++;
      else if (s[p] !== "}") fail(`expected ',' or '}' in flow map: ${s}`);
    }
  };
  const seq = () => {
    p++;
    const a = [];
    for (;;) {
      ws();
      if (s[p] === "]") {
        p++;
        return a;
      }
      a.push(value());
      ws();
      if (s[p] === ",") p++;
      else if (s[p] !== "]") fail(`expected ',' or ']' in flow sequence: ${s}`);
    }
  };
  const v = value();
  ws();
  if (p < s.length) fail(`trailing characters in flow value: ${s}`);
  return v;
}

function parseInline(str) {
  const t = str.trim();
  if (t[0] === "{" || t[0] === "[") return parseFlow(t);
  if (t[0] === '"' || t[0] === "'") {
    const end = closingQuote(t, 0);
    if (end !== t.length - 1) fail(`unexpected text after quoted scalar: ${t}`);
    return unquote(t);
  }
  return scalar(t);
}

function splitKey(content) {
  if (content[0] === "{" || content[0] === "[") return null;
  if (content[0] === '"' || content[0] === "'") {
    const end = closingQuote(content, 0);
    if (end < 0 || content[end + 1] !== ":") return null;
    return { key: unquote(content.slice(0, end + 1)), rest: content.slice(end + 2).trim() };
  }
  const m = content.search(/:(\s|$)/);
  if (m < 0) return null;
  return { key: content.slice(0, m).trim(), rest: content.slice(m + 1).trim() };
}

export function parseYaml(text) {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  let i = 0;
  const indentOf = (l) => l.length - l.trimStart().length;
  const textOf = (l) => stripComment(l).trim();
  const skip = () => {
    while (i < lines.length && (lines[i].trim() === "" || lines[i].trim().startsWith("#"))) i++;
  };

  function node(minIndent) {
    skip();
    if (i >= lines.length) return null;
    const line = lines[i];
    const ind = indentOf(line);
    if (ind < minIndent) return null;
    const content = textOf(line);
    if (content === "-" || content.startsWith("- ")) return seq(ind);
    if (splitKey(content)) return map(ind);
    i++;
    return parseInline(content);
  }

  function map(ind) {
    const out = {};
    for (;;) {
      skip();
      if (i >= lines.length) break;
      const line = lines[i];
      const cur = indentOf(line);
      if (cur < ind) break;
      if (cur > ind) fail(`unexpected indent at line ${i + 1}: ${line.trim()}`);
      const content = textOf(line);
      if (content === "-" || content.startsWith("- ")) break;
      const kv = splitKey(content);
      if (!kv) fail(`expected "key: value" at line ${i + 1}: ${line.trim()}`);
      i++;
      out[kv.key] = valueAfterKey(kv.rest, ind);
    }
    return out;
  }

  function valueAfterKey(rest, keyIndent) {
    if (rest === "") {
      skip();
      if (i < lines.length) {
        const ni = indentOf(lines[i]);
        if (ni > keyIndent) return node(ni);
        if (ni === keyIndent && textOf(lines[i]).startsWith("- ")) return seq(ni);
      }
      return null;
    }
    if (/^[|>][+-]?$/.test(rest)) {
      const folded = rest[0] === ">";
      const buf = [];
      while (i < lines.length && (lines[i].trim() === "" || indentOf(lines[i]) > keyIndent)) buf.push(lines[i++]);
      while (buf.length && buf[buf.length - 1].trim() === "") buf.pop();
      const base = Math.min(...buf.filter((l) => l.trim() !== "").map(indentOf));
      return buf.map((l) => l.slice(base)).join(folded ? " " : "\n");
    }
    return parseInline(rest);
  }

  function seq(ind) {
    const out = [];
    for (;;) {
      skip();
      if (i >= lines.length) break;
      const line = lines[i];
      if (indentOf(line) !== ind) break;
      const content = textOf(line);
      if (!(content === "-" || content.startsWith("- "))) break;
      const rest = content === "-" ? "" : content.slice(2).trim();
      if (rest === "") {
        i++;
        out.push(node(ind + 1));
      } else if (splitKey(rest)) {
        // "- key: value" opens a map whose entries align with the text after the dash.
        const col = line.indexOf(rest, ind + 1);
        lines[i] = " ".repeat(col) + rest;
        out.push(map(col));
      } else {
        i++;
        out.push(parseInline(rest));
      }
    }
    return out;
  }

  const doc = node(0);
  skip();
  if (i < lines.length) fail(`unparsed content at line ${i + 1}: ${lines[i].trim()}`);
  return doc;
}

// ---------------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------------

const BANNER = (what) =>
  `// AUTO-GENERATED by scripts/gen-sse.mjs from ../api/openapi.yaml (${what}).\n// Do not edit; run \`pnpm gen:types\`.\n`;

// Convenience aliases for components.schemas. [exportedName, schemaName]
const ALIASES = [
  ["Status", "Status"],
  ["ServiceRecord", "ServiceRecord"],
  ["ServiceId", "ServiceId"],
  ["ServiceState", "ServiceState"],
  ["ChildState", "ChildState"],
  ["DeviceRecord", "DeviceRecord"],
  ["DeviceKind", "DeviceKind"],
  ["DeviceState", "DeviceState"],
  ["DeviceTransport", "DeviceTransport"],
  ["Proposal", "Proposal"],
  ["Decision", "Decision"],
  ["Settings", "Settings"],
  ["SettingsPatch", "SettingsPatch"],
  ["AuthSession", "AuthSession"],
  ["ConversationId", "ConversationId"],
  ["ChatRequest", "ChatRequest"],
  ["ChatReply", "ChatReply"],
  ["ChatHistory", "ChatHistory"],
  ["ToolTrace", "ToolTrace"],
  ["AgentStatus", "AgentStatus"],
  ["TaskRequest", "TaskRequest"],
  ["LogTail", "LogTail"],
  ["Problem", "Problem"],
  // SSE payload schemas under friendlier names
  ["ChatToken", "SseChatToken"],
  ["ChatTool", "SseChatTool"],
  ["ChatDone", "SseChatDone"],
  ["ChatError", "SseChatError"],
  ["AgentEvent", "SseAgentEvent"],
  ["ProposalEvent", "SseProposal"],
  ["ResetEvent", "SseReset"],
  ["ServicesEvent", "SseServices"],
  ["DevicesEvent", "SseDevices"],
  // Layer 5 (Messaging & Data, os/50-messaging/README.md §15). `Panel` is exported as HostedPanel:
  // components/panel.tsx already owns the name `Panel` in the app.
  ["NodeId", "NodeId"],
  ["MessageId", "MessageId"],
  ["Message", "Message"],
  ["MessageState", "MessageState"],
  ["Thread", "Thread"],
  ["ThreadStates", "ThreadStates"],
  ["SendMessageRequest", "SendMessageRequest"],
  ["Contact", "Contact"],
  ["ContactTrust", "ContactTrust"],
  ["ContactCreate", "ContactCreate"],
  ["ContactPatch", "ContactPatch"],
  ["QrMatrix", "QrMatrix"],
  ["Card", "Card"],
  ["LinkKind", "LinkKind"],
  ["LinkState", "LinkState"],
  ["FabricPathProfile", "FabricPathProfile"],
  ["FabricLink", "FabricLink"],
  ["FabricHub", "FabricHub"],
  ["FabricCustody", "FabricCustody"],
  ["FabricStore", "FabricStore"],
  ["FabricSettings", "FabricSettings"],
  ["FabricSettingsPatch", "FabricSettingsPatch"],
  ["FabricStatus", "FabricStatus"],
  ["HubEnrollRequest", "HubEnrollRequest"],
  ["HubOffer", "HubOffer"],
  ["NearbyNode", "NearbyNode"],
  ["NearbySource", "NearbySource"],
  ["HostedPanel", "Panel"],
  ["PanelState", "PanelState"],
  ["PanelSession", "PanelSession"],
  ["IdentifyRequest", "IdentifyRequest"],
  // Layer 5 SSE payloads (the `fabric` frame carries a FabricStatus snapshot directly)
  ["MessageChangeEvent", "SseMessage"],
  ["ContactChangeEvent", "SseContact"],
  ["NearbyEvent", "SseNearby"],
  ["PanelsEvent", "SsePanels"],
];

// Runtime enum tuples. [exportedName, schemaName, path inside the schema to the enum array]
const ENUMS = [
  ["SERVICE_IDS", "ServiceId", []],
  ["SERVICE_STATES", "ServiceState", []],
  ["CHILD_STATES", "ChildState", []],
  ["DEVICE_KINDS", "DeviceKind", []],
  ["DEVICE_STATES", "DeviceState", []],
  ["DEVICE_TRANSPORTS", "DeviceTransport", []],
  ["DECISIONS", "Decision", []],
  ["PROBLEM_CODES", "Problem", ["properties", "code"]],
  ["DEVICE_ACTIONS", "DeviceRecord", ["properties", "actions", "items"]],
  // Layer 5
  ["MESSAGE_STATES", "MessageState", []],
  ["MESSAGE_DIRS", "Message", ["properties", "dir"]],
  ["CONTACT_TRUSTS", "ContactTrust", []],
  ["LINK_KINDS", "LinkKind", []],
  ["LINK_STATES", "LinkState", []],
  ["NEARBY_KINDS", "NearbySource", ["properties", "kind"]],
  ["PANEL_HEALTHS", "PanelState", ["properties", "health"]],
  ["MESSAGE_CHANGES", "SseMessage", ["properties", "change"]],
  ["CONTACT_CHANGES", "SseContact", ["properties", "change"]],
];

function schemaRefName(ref, schemas) {
  const m = /^#\/components\/schemas\/([A-Za-z0-9_]+)$/.exec(String(ref));
  if (!m || !(m[1] in schemas)) fail(`x-sse-events references unknown schema: ${ref}`);
  return m[1];
}

export function renderSse(doc) {
  const schemas = doc?.components?.schemas ?? fail("components.schemas missing");
  const sse = doc["x-sse-events"] ?? fail("x-sse-events missing");
  const events = sse.events ?? fail("x-sse-events.events missing");
  const chat = sse.chat ?? fail("x-sse-events.chat missing");

  const union = (map) =>
    Object.entries(map)
      .map(([name, ref]) => `  | { event: ${JSON.stringify(name)}; data: Schemas[${JSON.stringify(schemaRefName(ref, schemas))}] }`)
      .join("\n");
  const tuple = (names) => `[${names.map((n) => JSON.stringify(n)).join(", ")}] as const`;

  let out = BANNER("x-sse-events + components.schemas");
  out += `\nimport type { components } from "./types.gen";\n\ntype Schemas = components["schemas"];\n`;

  out += `\n/** Frames on \`GET /api/v1/events\` (named SSE events; \`id\` rides alongside, see the adapter). */\n`;
  out += `export type EventsFrame =\n${union(events)};\n`;
  out += `\n/** Frames on the \`POST /api/v1/chat\` response stream. */\n`;
  out += `export type ChatFrame =\n${union(chat)};\n`;
  out += `\nexport type EventsEventName = EventsFrame["event"];\nexport type ChatEventName = ChatFrame["event"];\n`;
  out += `\nexport const EVENTS_EVENT_NAMES = ${tuple(Object.keys(events))};\n`;
  out += `export const CHAT_EVENT_NAMES = ${tuple(Object.keys(chat))};\n`;

  out += `\n// Convenience aliases for the schemas the app uses.\n`;
  for (const [alias, name] of ALIASES) {
    if (!(name in schemas)) fail(`alias ${alias}: schema ${name} not found in the contract`);
    out += `export type ${alias} = Schemas[${JSON.stringify(name)}];\n`;
  }
  out += `export type ProblemCode = Schemas["Problem"]["code"];\n`;

  out += `\n// Enum tuples (runtime copies of the contract enums).\n`;
  for (const [exported, schemaName, path] of ENUMS) {
    let node = schemas[schemaName] ?? fail(`enum ${exported}: schema ${schemaName} not found`);
    for (const key of path) node = node?.[key];
    if (!Array.isArray(node?.enum)) fail(`enum ${exported}: no enum array at ${schemaName}.${path.join(".")}`);
    out += `export const ${exported} = ${tuple(node.enum)};\n`;
  }
  return out;
}

const METHODS = ["get", "post", "put", "patch", "delete"];

function jsonSchemaOf(content) {
  if (!content || typeof content !== "object") return { contentType: null, schema: null, byType: {} };
  const types = Object.keys(content);
  const byType = Object.fromEntries(types.map((t) => [t, content[t]?.schema ?? null]));
  return { contentType: types[0] ?? null, schema: types.length ? byType[types[0]] : null, byType };
}

export function buildContract(doc) {
  const schemas = doc?.components?.schemas ?? fail("components.schemas missing");
  const params = doc.components.parameters ?? {};
  const operations = {};
  for (const [path, item] of Object.entries(doc.paths ?? {})) {
    for (const method of METHODS) {
      const op = item?.[method];
      if (!op) continue;
      const id = op.operationId ?? fail(`${method} ${path}: operationId missing`);
      const resolved = (op.parameters ?? []).map((p) => {
        if (p?.$ref) {
          const name = String(p.$ref).split("/").pop();
          return params[name] ?? fail(`${id}: unknown parameter ${p.$ref}`);
        }
        return p;
      });
      const okStatus = Object.keys(op.responses ?? {}).find((s) => s.startsWith("2")) ?? fail(`${id}: no 2xx response`);
      const ok = jsonSchemaOf(op.responses[okStatus]?.content);
      const req = jsonSchemaOf(op.requestBody?.content);
      operations[id] = {
        method: method.toUpperCase(),
        path,
        success: Number(okStatus),
        contentType: ok.contentType,
        schema: ok.schema,
        schemasByType: ok.byType,
        requestSchema: req.schema,
        query: resolved.filter((p) => p.in === "query").map((p) => ({ name: p.name, required: p.required === true })),
        pathParams: resolved.filter((p) => p.in === "path").map((p) => p.name),
        errors: Object.keys(op.responses).filter((s) => !s.startsWith("2")).map(Number),
      };
    }
  }
  const sse = doc["x-sse-events"] ?? {};
  const names = (m) => Object.fromEntries(Object.entries(m ?? {}).map(([k, ref]) => [k, schemaRefName(ref, schemas)]));
  return { schemas, operations, sse: { events: names(sse.events), chat: names(sse.chat) } };
}

// JSON with short sub-trees kept on one line, so the checked-in digest stays reviewable.
function compactJson(value, indent = 0, width = 110) {
  const flat = JSON.stringify(value);
  if (flat === undefined || flat.length + indent <= width || typeof value !== "object" || value === null) return flat;
  const pad = " ".repeat(indent + 2);
  const end = " ".repeat(indent);
  if (Array.isArray(value)) return `[\n${value.map((v) => pad + compactJson(v, indent + 2, width)).join(",\n")}\n${end}]`;
  const rows = Object.entries(value).map(([k, v]) => `${pad}${JSON.stringify(k)}: ${compactJson(v, indent + 2, width)}`);
  return `{\n${rows.join(",\n")}\n${end}}`;
}

export function renderContract(doc) {
  const { schemas, operations, sse } = buildContract(doc);
  let out = BANNER("components.schemas + paths");
  out += `// Test-only digest: imported by vitest to validate fixtures and adapter calls. Not part of the app bundle.\n\n`;
  out += `export type JsonSchema = { [key: string]: unknown };\n\n`;
  out += `export interface ContractOperation {\n  method: string;\n  path: string;\n  success: number;\n  contentType: string | null;\n  schema: JsonSchema | null;\n  schemasByType: Record<string, JsonSchema | null>;\n  requestSchema: JsonSchema | null;\n  query: { name: string; required: boolean }[];\n  pathParams: string[];\n  errors: number[];\n}\n\n`;
  out += `export const schemas: Record<string, JsonSchema> = ${compactJson(schemas)};\n\n`;
  out += `export const operations: Record<string, ContractOperation> = ${compactJson(operations)};\n\n`;
  out += `/** SSE event name -> payload schema name, from x-sse-events. */\nexport const sse: { events: Record<string, string>; chat: Record<string, string> } = ${compactJson(sse)};\n`;
  return out;
}

// ---------------------------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------------------------

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const yamlPath = resolve(root, "../api/openapi.yaml");
  const targets = [
    { file: resolve(root, "src/lib/api/sse.gen.ts"), render: renderSse },
    { file: resolve(root, "test/contract.gen.ts"), render: renderContract },
  ];
  const doc = parseYaml(readFileSync(yamlPath, "utf8"));
  const check = process.argv.includes("--check");
  let stale = false;
  for (const { file, render } of targets) {
    const next = render(doc);
    const prev = existsSync(file) ? readFileSync(file, "utf8") : null;
    if (prev === next) continue;
    if (check) {
      stale = true;
      console.error(`stale: ${file}`);
    } else {
      writeFileSync(file, next);
      console.log(`wrote ${file}`);
    }
  }
  if (check && stale) {
    console.error("generated files are out of date with api/openapi.yaml; run `pnpm gen:types`");
    process.exit(1);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(e instanceof Error ? e.message : e);
    process.exit(1);
  }
}
