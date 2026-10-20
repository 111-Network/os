// A small JSON-Schema-subset validator over the digest generated from api/openapi.yaml.
// Supports what the contract uses: $ref, type (incl. type arrays with "null"), enum, const,
// required, properties, items, minimum/maximum, minLength/maxLength, pattern, format date-time, and oneOf (valid when ANY branch is
// valid: the contract uses it for "a schema or null" and for the fan-out target union, never for exclusive alternatives).
import { schemas, type JsonSchema } from "../contract.gen";

const DATE_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

function typeOk(value: unknown, t: string): boolean {
  switch (t) {
    case "string":
      return typeof value === "string";
    case "integer":
      return typeof value === "number" && Number.isInteger(value);
    case "number":
      return typeof value === "number" && Number.isFinite(value);
    case "boolean":
      return typeof value === "boolean";
    case "null":
      return value === null;
    case "array":
      return Array.isArray(value);
    case "object":
      return typeof value === "object" && value !== null && !Array.isArray(value);
    default:
      return false;
  }
}

export function resolveRef(ref: string): JsonSchema {
  const m = /^#\/components\/schemas\/(.+)$/.exec(ref);
  const s = m ? schemas[m[1]!] : undefined;
  if (!s) throw new Error(`unresolvable $ref ${ref}`);
  return s;
}

export function validate(value: unknown, schema: JsonSchema, path = "$"): string[] {
  if (typeof schema.$ref === "string") return validate(value, resolveRef(schema.$ref), path);
  const errors: string[] = [];
  if (Array.isArray(schema.oneOf)) {
    const failures = (schema.oneOf as JsonSchema[]).map((branch) => validate(value, branch, path));
    if (!failures.some((f) => f.length === 0)) {
      return [`${path}: matches none of ${failures.length} oneOf branches (${failures.map((f) => f[0] ?? "").join(" | ")})`];
    }
  }
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? (schema.type as string[]) : [schema.type as string];
    if (!types.some((t) => typeOk(value, t))) {
      return [`${path}: expected ${types.join("|")}, got ${value === null ? "null" : Array.isArray(value) ? "array" : typeof value}`];
    }
  }
  if (Array.isArray(schema.enum) && !(schema.enum as unknown[]).includes(value)) {
    errors.push(`${path}: ${JSON.stringify(value)} not in enum [${(schema.enum as unknown[]).join(", ")}]`);
  }
  if ("const" in schema && schema.const !== value) errors.push(`${path}: expected const ${JSON.stringify(schema.const)}`);
  if (typeof value === "number") {
    if (typeof schema.minimum === "number" && value < schema.minimum) errors.push(`${path}: below minimum ${schema.minimum}`);
    if (typeof schema.maximum === "number" && value > schema.maximum) errors.push(`${path}: above maximum ${schema.maximum}`);
  }
  if (typeof value === "string") {
    if (typeof schema.minLength === "number" && value.length < schema.minLength) errors.push(`${path}: shorter than ${schema.minLength}`);
    if (typeof schema.maxLength === "number" && value.length > schema.maxLength) errors.push(`${path}: longer than ${schema.maxLength}`);
    if (typeof schema.pattern === "string" && !new RegExp(schema.pattern).test(value)) errors.push(`${path}: does not match ${schema.pattern}`);
    if (schema.format === "date-time" && !DATE_TIME.test(value)) errors.push(`${path}: not an RFC 3339 date-time`);
  }
  if (Array.isArray(value) && schema.items) {
    value.forEach((v, i) => errors.push(...validate(v, schema.items as JsonSchema, `${path}[${i}]`)));
  }
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>;
    for (const key of (schema.required as string[] | undefined) ?? []) if (!(key in obj)) errors.push(`${path}: missing required "${key}"`);
    const props = (schema.properties as Record<string, JsonSchema> | undefined) ?? {};
    for (const [k, sub] of Object.entries(props)) if (k in obj) errors.push(...validate(obj[k], sub, `${path}.${k}`));
  }
  return errors;
}

export function validateAgainst(name: string, value: unknown): string[] {
  return validate(value, { $ref: `#/components/schemas/${name}` });
}
