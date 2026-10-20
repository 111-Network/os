// The size gate (spec section 2, D21): the Layer 6 views are lazy chunks and each must stay at or under 64 KB gzip. This drives
// scripts/check-budget.mjs against small throwaway dist folders, so the cap that guards /messages/net and /settings/l6 is itself guarded.
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const SCRIPT = resolve(__dirname, "..", "scripts", "check-budget.mjs");
const dirs: string[] = [];
afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

/** A dist with an eager entry and one lazy chunk of `lazyBytes` of incompressible data (so its gzip size is about its raw size). */
function dist(lazyBytes: number): string {
  const d = mkdtempSync(join(tmpdir(), "budget-"));
  dirs.push(d);
  mkdirSync(join(d, "assets"));
  writeFileSync(join(d, "index.html"), '<!doctype html><script type="module" src="./assets/entry.js"></script>');
  writeFileSync(join(d, "assets", "entry.js"), "export const a = 1;\n// __reactFiber$\n");
  writeFileSync(join(d, "assets", "net.js"), randomBytes(lazyBytes));
  return d;
}

const run = (d: string) => spawnSync(process.execPath, [SCRIPT, "--dist", d], { encoding: "utf8" });

describe("scripts/check-budget.mjs", () => {
  it("passes a lazy chunk under 64 KB gzip", () => {
    const r = run(dist(40_000));
    expect(r.status, r.stdout + r.stderr).toBe(0);
    expect(r.stdout).toContain("PASS");
  });

  it("fails a lazy chunk over 64 KB gzip and names it", () => {
    const r = run(dist(70_000));
    expect(r.status).toBe(1);
    expect(r.stderr).toContain("lazy chunk assets/net.js");
    expect(r.stderr).toContain("per-chunk cap");
  });
});
