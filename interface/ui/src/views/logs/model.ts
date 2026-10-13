// Pure model for the Logs view (spec section 6.8): which services get a tab, how a tail is cleaned
// and described. No React, so it runs in the node test environment.
import type { LogTail, ServiceRecord } from "../../lib/api/types";
import { sanitizeLine } from "../../lib/prompts";

/** Lines requested per tail (the contract allows up to 500). */
export const LOG_LINES = 200;
/** Poll period while Follow is on. */
export const LOG_POLL_MS = 5000;

export const JOURNAL_NOTE = "The daemon's own log is in the system journal — on the node run";
export const JOURNAL_COMMAND = "journalctl --namespace=111node -f";

/** Services whose registry record offers the `log` action, in registry order. */
export function logServices(services: readonly ServiceRecord[] | null | undefined): ServiceRecord[] {
  return (services ?? []).filter((s) => s.actions.includes("log"));
}

/** Log lines are untrusted text: drop control and bidi characters before showing or copying them. */
export function cleanLines(lines: readonly string[] | null | undefined): string[] {
  return (lines ?? []).map(sanitizeLine);
}

/** The tail reports no log source at all (not merely an empty file). */
export function hasNoSource(tail: LogTail | null | undefined): boolean {
  return tail?.source === "none";
}

/** "200 lines · source /var/lib/111/log/111-agent.log" */
export function tailSummary(tail: LogTail | null | undefined): string {
  if (!tail) return "no tail loaded yet";
  const n = tail.lines.length;
  return `${n} ${n === 1 ? "line" : "lines"} · source ${tail.source}`;
}

/** What the terminal says when it has no lines to show. */
export function emptyLogText(tail: LogTail | null | undefined, loading: boolean): string {
  if (!tail) return loading ? "Loading the log…" : "No log could be loaded.";
  return "The log file is empty.";
}

/** Follow state in words, for the line under the terminal. */
export function followText(follow: boolean): string {
  return follow ? `following, refreshed every ${LOG_POLL_MS / 1000} s` : "not following";
}
