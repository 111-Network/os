import { describe, expect, it } from "vitest";
import { ApiError, asProblem, errorFromResponse, isApiError, isProblem } from "../src/lib/api/problem";
import { jsonResponse, problemResponse } from "./helpers/fakes";

describe("isProblem", () => {
  it("requires the four contract fields", () => {
    expect(isProblem({ type: "about:blank", title: "t", status: 503, code: "agent_unavailable" })).toBe(true);
    expect(isProblem({ title: "t", status: 503, code: "x" })).toBe(false);
    expect(isProblem({ type: "a", title: "t", status: "503", code: "x" })).toBe(false);
    expect(isProblem(null)).toBe(false);
    expect(isProblem("nope")).toBe(false);
  });
});

describe("asProblem", () => {
  it("returns the server's problem+json body", async () => {
    const p = await asProblem(problemResponse(503, "model_unavailable", "Model down", "llama-server is not running"));
    expect(p).toMatchObject({ status: 503, code: "model_unavailable", title: "Model down", detail: "llama-server is not running", type: "about:blank" });
    expect(isProblem(p)).toBe(true);
  });

  it("keeps field-level errors", async () => {
    const res = jsonResponse({ type: "about:blank", title: "Bad", status: 400, code: "bad_request", errors: [{ pointer: "/alias", message: "too long" }] }, 400);
    expect((await asProblem(res)).errors).toEqual([{ pointer: "/alias", message: "too long" }]);
  });

  it("fills a missing type and fixes a missing status from the response", async () => {
    const res = jsonResponse({ title: "Nope", code: "not_found" }, 404);
    expect(await asProblem(res)).toMatchObject({ type: "about:blank", status: 404, code: "not_found" });
  });

  it("synthesizes a problem from plain text", async () => {
    const res = new Response("upstream sad", { status: 502, statusText: "Bad Gateway" });
    expect(await asProblem(res)).toMatchObject({ status: 502, title: "Bad Gateway", code: "upstream_error", detail: "upstream sad" });
  });

  it("does not leak an HTML error page into detail", async () => {
    const res = new Response("<html><body>502</body></html>", { status: 502 });
    const p = await asProblem(res);
    expect(p.detail).toBeUndefined();
    expect(p.code).toBe("upstream_error");
  });

  it("handles an empty body", async () => {
    expect(await asProblem(new Response(null, { status: 500 }))).toMatchObject({ status: 500, code: "internal", title: "HTTP 500" });
  });

  it("always maps 401 to unauthenticated, whatever the body says", async () => {
    expect((await asProblem(new Response("", { status: 401 }))).code).toBe("unauthenticated");
    expect((await asProblem(problemResponse(401, "internal"))).code).toBe("unauthenticated");
  });
});

describe("ApiError", () => {
  it("carries status, code and the problem", async () => {
    const e = await errorFromResponse(problemResponse(409, "already_decided", "Already decided", "It was accepted."));
    expect(isApiError(e)).toBe(true);
    expect(e).toBeInstanceOf(Error);
    expect(e).toMatchObject({ name: "ApiError", status: 409, code: "already_decided", message: "It was accepted." });
    expect(e.problem.title).toBe("Already decided");
    expect(e.unauthenticated).toBe(false);
  });

  it("represents a network failure with status 0", () => {
    const cause = new TypeError("Failed to fetch");
    const e = ApiError.network(cause);
    expect(e).toMatchObject({ status: 0, code: "network", message: "Failed to fetch" });
    expect(e.cause).toBe(cause);
    expect(isProblem(e.problem)).toBe(true);
  });
});
