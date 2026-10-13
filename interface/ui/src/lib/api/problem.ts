// RFC 9457 problem+json handling. Every adapter method rejects with ApiError.
import type { Problem, ProblemCode } from "./types";

/** `problem.code`, or `network` when the request never produced an HTTP response. */
export type ApiErrorCode = ProblemCode | "network";

export class ApiError extends Error {
  readonly status: number;
  readonly code: ApiErrorCode;
  readonly problem: Problem;
  cause?: unknown;

  constructor(problem: Problem, options?: { cause?: unknown; code?: ApiErrorCode }) {
    super(problem.detail || problem.title);
    this.name = "ApiError";
    if (options?.cause !== undefined) this.cause = options.cause; // ES2022 `cause` option is outside our lib target
    this.status = problem.status;
    this.code = options?.code ?? problem.code;
    this.problem = problem;
  }

  /** The request failed before any HTTP response (offline, refused, DNS, reset). */
  static network(cause: unknown): ApiError {
    const detail = cause instanceof Error ? cause.message : String(cause);
    return new ApiError(
      { type: "about:blank", title: "Node unreachable", status: 0, code: "upstream_error", detail },
      { cause, code: "network" },
    );
  }

  get unauthenticated(): boolean {
    return this.code === "unauthenticated";
  }
}

export function isApiError(x: unknown): x is ApiError {
  return x instanceof ApiError;
}

/** Strict: has every field the contract marks required for a Problem. */
export function isProblem(x: unknown): x is Problem {
  if (typeof x !== "object" || x === null) return false;
  const p = x as Record<string, unknown>;
  return (
    typeof p.type === "string" &&
    typeof p.title === "string" &&
    typeof p.status === "number" &&
    typeof p.code === "string"
  );
}

function codeForStatus(status: number): ProblemCode {
  switch (status) {
    case 400:
      return "bad_request";
    case 401:
      return "unauthenticated";
    case 404:
      return "not_found";
    case 429:
      return "rate_limited";
    case 502:
    case 503:
    case 504:
      return "upstream_error";
    default:
      return "internal";
  }
}

/** A Problem for a response that did not carry one (proxy error page, plain text, empty body). */
export function synthProblem(status: number, statusText: string, detail?: string): Problem {
  const p: Problem = {
    type: "about:blank",
    title: statusText || `HTTP ${status}`,
    status,
    code: codeForStatus(status),
  };
  if (detail) p.detail = detail;
  return p;
}

/**
 * Read an error response into a Problem. Never throws. A 401 always maps to `unauthenticated`
 * (the shell routes to /pair), whatever the body says.
 */
export async function asProblem(response: Response): Promise<Problem> {
  let text = "";
  try {
    text = await response.text();
  } catch {
    /* body unreadable: fall through to a synthesized problem */
  }
  let problem: Problem | null = null;
  if (text) {
    try {
      const body: unknown = JSON.parse(text);
      if (typeof body === "object" && body !== null) {
        const b = body as Partial<Problem>;
        if (typeof b.title === "string" || typeof b.code === "string") {
          const status = typeof b.status === "number" ? b.status : response.status;
          problem = {
            ...(b as Problem),
            type: typeof b.type === "string" ? b.type : "about:blank",
            title: typeof b.title === "string" ? b.title : response.statusText || `HTTP ${status}`,
            status,
            code: typeof b.code === "string" ? b.code : codeForStatus(status),
          };
        }
      }
    } catch {
      /* not JSON */
    }
  }
  if (!problem) {
    const plain = text.trim().startsWith("<") ? "" : text.trim().slice(0, 300);
    problem = synthProblem(response.status, response.statusText, plain || undefined);
  }
  if (response.status === 401) problem.code = "unauthenticated";
  return problem;
}

/** Convenience: build the ApiError for an error response. */
export async function errorFromResponse(response: Response): Promise<ApiError> {
  return new ApiError(await asProblem(response));
}
