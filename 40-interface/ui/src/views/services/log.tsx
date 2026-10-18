// Log tail for a service: fetched through the adapter, optionally polled while mounted.
import { useCallback, useEffect, useState } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { Button } from "../../components/button";
import { MiniTerminal } from "../../components/mini-terminal";
import { getAdapter, isApiError } from "../../lib/api";
import type { ServiceId } from "../../lib/api/types";
import { sanitizeLine } from "../../lib/prompts";

export const LOG_POLL_MS = 5000;

interface LogState {
  lines: string[];
  source: string | null;
  error: string | null;
  loaded: boolean;
}

/** One fetch, no polling, no state: for the prompt context. Failures give []. */
export async function fetchLogLines(id: ServiceId, n: number): Promise<string[]> {
  try {
    const adapter = await getAdapter();
    return (await adapter.serviceLog(id, n)).lines;
  } catch (err) {
    redirectIfUnauthenticated(err);
    return [];
  }
}

/**
 * Tail of `n` lines, refreshed every `pollMs` (0 = once) while mounted. Requests never overlap and are
 * skipped while the tab is hidden. `refresh()` fetches now.
 */
export function useServiceLog(id: ServiceId, n: number, pollMs: number) {
  const [state, setState] = useState<LogState>({ lines: [], source: null, error: null, loaded: false });
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let alive = true;
    let timer: number | undefined;
    const load = async () => {
      if (!document.hidden || tick > 0) {
        try {
          const adapter = await getAdapter();
          const tail = await adapter.serviceLog(id, n);
          if (alive) setState({ lines: tail.lines.map(sanitizeLine), source: tail.source, error: null, loaded: true });
        } catch (err) {
          if (alive && !redirectIfUnauthenticated(err)) {
            setState((s) => ({ ...s, loaded: true, error: isApiError(err) ? err.message : "Could not read the log." }));
          }
        }
      }
      if (alive && pollMs > 0) timer = window.setTimeout(() => void load(), pollMs);
    };
    void load();
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [id, n, pollMs, tick]);

  const refresh = useCallback(() => setTick((t) => t + 1), []);
  return { ...state, refresh };
}

export interface LogTailProps {
  id: ServiceId;
  title: string;
  lines: number;
  /** Refresh interval in ms; 0 fetches once. */
  pollMs: number;
  /** Show the refresh button and the source line. */
  controls?: boolean;
  maxHeightClass?: string;
}

export function LogTail({ id, title, lines, pollMs, controls = false, maxHeightClass }: LogTailProps) {
  const log = useServiceLog(id, lines, pollMs);
  return (
    <div className="space-y-2">
      <MiniTerminal
        title={title}
        lines={log.lines}
        follow
        live={pollMs > 0 && !log.error}
        maxHeightClass={maxHeightClass}
        emptyText={log.error ? undefined : log.loaded ? "No log lines yet." : "Reading the log…"}
      />
      {log.error ? (
        <p role="alert" className="font-mono text-xs text-destructive">
          {log.error}
        </p>
      ) : null}
      {controls ? (
        <div className="flex flex-wrap items-center gap-3 font-mono text-xs text-muted-foreground">
          <Button size="sm" onClick={log.refresh}>
            Refresh
          </Button>
          <span>{pollMs > 0 ? `updates every ${pollMs / 1000} s` : null}</span>
          {log.source ? <span className="break-all">source: {log.source}</span> : null}
        </div>
      ) : null}
    </div>
  );
}
