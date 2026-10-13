import { afterEach, describe, expect, it, vi } from "vitest";
import { getAdapter, resetAdapterForTests, selectAdapter } from "../src/lib/api";

afterEach(() => {
  vi.unstubAllEnvs();
  resetAdapterForTests();
});

describe("selectAdapter", () => {
  it("defaults to the http adapter", async () => {
    expect((await selectAdapter("")).kind).toBe("http");
    expect((await selectAdapter("?scenario=first-run")).kind).toBe("http");
    expect((await selectAdapter("?adapter=http")).kind).toBe("http");
    expect((await selectAdapter("?adapter=bogus")).kind).toBe("http");
  });

  it("picks the mock only with ?adapter=mock in dev, honouring ?scenario=", async () => {
    const a = await selectAdapter("?adapter=mock&scenario=agent-down");
    expect(a.kind).toBe("mock");
    expect((await a.service("agent")).state).toBe("error");
    (a as unknown as { mock: { dispose(): void } }).mock.dispose();
  });

  it("falls back to the healthy scenario for an unknown name", async () => {
    const a = await selectAdapter("?adapter=mock&scenario=nope");
    expect((await a.service("agent")).state).toBe("live");
    (a as unknown as { mock: { dispose(): void } }).mock.dispose();
  });

  it("never picks the mock in production", async () => {
    vi.stubEnv("DEV", false);
    expect((await selectAdapter("?adapter=mock")).kind).toBe("http");
  });

  it("getAdapter memoizes", async () => {
    const first = getAdapter();
    expect(getAdapter()).toBe(first);
    expect((await first).kind).toBe("http");
  });
});
