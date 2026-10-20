import { describe, expect, it } from "vitest";
import { consumeOnce, readParams } from "../src/lib/params";

describe("readParams", () => {
  it("reads the document query", () => {
    const p = readParams("?mode=advanced&x=1", "#/overview");
    expect(p.get("mode")).toBe("advanced");
    expect(p.get("x")).toBe("1");
  });

  it("reads an in-hash query", () => {
    const p = readParams("", "#/pair?code=ABCD-EFGH");
    expect(p.get("code")).toBe("ABCD-EFGH");
  });

  it("merges both, and the document wins on conflict", () => {
    const p = readParams("?mode=simple&a=doc", "#/settings?mode=advanced&b=hash");
    expect(p.get("mode")).toBe("simple");
    expect(p.get("a")).toBe("doc");
    expect(p.get("b")).toBe("hash");
  });

  it("copes with empty and odd input", () => {
    expect([...readParams("", "")]).toEqual([]);
    expect([...readParams("?", "#")]).toEqual([]);
    expect([...readParams("", "#/route")]).toEqual([]);
    expect(readParams("", "#?p=v").get("p")).toBe("v");
    expect(readParams("p=v", "").get("p")).toBe("v");
  });
});

describe("consumeOnce", () => {
  function run(name: string, search: string, hash?: string) {
    const calls: [string, string][] = [];
    const value = consumeOnce(name, { search, ...(hash === undefined ? {} : { hash }), replace: (s, h) => calls.push([s, h]) });
    return { value, calls };
  }

  it("returns null and does not touch the URL when the parameter is absent", () => {
    expect(run("mode", "?a=1", "#/x?b=2")).toEqual({ value: null, calls: [] });
    expect(run("mode", "")).toEqual({ value: null, calls: [] });
  });

  it("returns the value and strips it from the document query, keeping other params", () => {
    expect(run("mode", "?mode=advanced&a=1", "#/overview")).toEqual({ value: "advanced", calls: [["?a=1", "#/overview"]] });
  });

  it("strips the last parameter down to an empty search", () => {
    expect(run("mode", "?mode=advanced", "#/overview").calls).toEqual([["", "#/overview"]]);
  });

  it("strips a hash-only parameter and keeps the route", () => {
    expect(run("code", "", "#/pair?code=ABCD-EFGH")).toEqual({ value: "ABCD-EFGH", calls: [["", "#/pair"]] });
    expect(run("code", "?x=1", "#/pair?code=ABCD&y=2").calls).toEqual([["?x=1", "#/pair?y=2"]]);
  });

  it("prefers the document value but removes the name from both places", () => {
    expect(run("mode", "?mode=simple", "#/s?mode=advanced&k=v")).toEqual({ value: "simple", calls: [["", "#/s?k=v"]] });
  });

  it("is one-shot: re-reading the rewritten URL yields nothing", () => {
    const first = run("mode", "?mode=advanced&a=1", "#/overview?z=9");
    const [search, hash] = first.calls[0]!;
    expect(run("mode", search, hash).value).toBeNull();
    expect(readParams(search, hash).get("a")).toBe("1");
    expect(readParams(search, hash).get("z")).toBe("9");
  });

  it("accepts a hash without its leading # and always writes one back", () => {
    expect(run("p", "", "/route?p=v").calls).toEqual([["", "#/route"]]);
    expect(run("p", "?p=v", "").calls).toEqual([["", ""]]);
  });

  it("decodes percent-escapes", () => {
    expect(run("prompt", "?prompt=fix%20the%20radio").value).toBe("fix the radio");
  });
});
