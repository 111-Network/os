// Logs (spec section 6.8, advanced): one tab per service that keeps a log, a live tail with Follow,
// copy, and the note that the daemon's own log lives in the journal.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo } from "../../app/location";
import { Button, ButtonLink } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Empty } from "../../components/empty";
import { MiniTerminal } from "../../components/mini-terminal";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { Segmented } from "../../components/segmented";
import { Toggle } from "../../components/toggle";
import { getAdapter } from "../../lib/api";
import { useServices } from "../../lib/api/hooks";
import type { LogTail, ServiceId } from "../../lib/api/types";
import { problemText } from "../settings/model";
import { JOURNAL_COMMAND, JOURNAL_NOTE, LOG_LINES, LOG_POLL_MS, cleanLines, emptyLogText, followText, hasNoSource, logServices, tailSummary } from "./model";

interface TailState {
  /** The service this state belongs to; a tab switch shows nothing stale. */
  id: ServiceId | null;
  tail: LogTail | null;
  error: string | null;
  loading: boolean;
  at: number | null;
}

const NONE: TailState = { id: null, tail: null, error: null, loading: false, at: null };

/**
 * Loads the tail for `id` on selection and on `refresh()`, and every LOG_POLL_MS while `follow` is
 * on (skipped while the tab is hidden). A slow older response never replaces a newer one.
 */
function useLogTail(id: ServiceId | undefined, follow: boolean) {
  const [state, setState] = useState<TailState>(NONE);
  const idRef = useRef(id);
  const issued = useRef(0);
  const applied = useRef(0);
  useEffect(() => {
    idRef.current = id;
  }, [id]);

  const load = useCallback(async (serviceId: ServiceId) => {
    const mine = ++issued.current;
    try {
      const tail = await (await getAdapter()).serviceLog(serviceId, LOG_LINES);
      if (idRef.current !== serviceId || mine < applied.current) return;
      applied.current = mine;
      setState({ id: serviceId, tail, error: null, loading: false, at: Date.now() });
    } catch (err) {
      if (idRef.current !== serviceId || redirectIfUnauthenticated(err)) return;
      // Keep the last good tail on screen; say what went wrong next to it.
      setState((s) => ({ id: serviceId, tail: s.id === serviceId ? s.tail : null, error: problemText(err), loading: false, at: s.id === serviceId ? s.at : null }));
    }
  }, []);

  useEffect(() => {
    if (!id) return;
    setState((s) => (s.id === id ? s : { ...NONE, id, loading: true }));
    void load(id);
  }, [id, load]);

  useEffect(() => {
    if (!id || !follow) return;
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void load(id);
    }, LOG_POLL_MS);
    return () => window.clearInterval(timer);
  }, [id, follow, load]);

  // Until the effect above has switched tabs, show "loading" rather than the previous tab's lines.
  const current = state.id === id ? state : { ...NONE, loading: id !== undefined };
  return { ...current, refresh: () => (id ? void load(id) : undefined) };
}

export default function LogsView() {
  const services = useServices();
  const tabs = useMemo(() => logServices(services), [services]);
  const [picked, setPicked] = useState<ServiceId | null>(null);
  const [follow, setFollow] = useState(true);
  const current = tabs.find((s) => s.id === picked) ?? tabs[0];
  const { tail, error, loading, at, refresh } = useLogTail(current?.id, follow);

  const lines = useMemo(() => cleanLines(tail?.lines), [tail]);
  const noSource = hasNoSource(tail);

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader level={1} kicker="advanced" title="Logs" />

      <Panel title="journal" suffix="daemon">
        <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">{JOURNAL_NOTE}</p>
        <div className="mt-3 flex flex-wrap items-center gap-3">
          <code className="min-w-0 whitespace-pre-wrap break-words border border-border bg-background px-3 py-2 font-mono text-xs text-foreground">{JOURNAL_COMMAND}</code>
          <CopyButton text={JOURNAL_COMMAND} ariaLabel="Copy the journalctl command" />
        </div>
      </Panel>

      {services === null ? (
        <Empty kicker="waiting" title="Waiting for the node">
          The service list has not arrived yet.
        </Empty>
      ) : !current ? (
        <Empty kicker="no logs" title="No service keeps a log here">
          None of this node's services offer a log tail. The daemon's own log is in the journal, above.
        </Empty>
      ) : (
        <section aria-label="Service logs" className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <div className="max-w-full overflow-x-auto">
              <Segmented<ServiceId>
                label="Log source"
                value={current.id}
                onChange={setPicked}
                options={tabs.map((s) => ({ value: s.id, label: s.plain_name }))}
              />
            </div>
            <Toggle pressed={follow} onPressedChange={setFollow} className="px-4 py-2 text-xs">
              <span>Follow</span> <span aria-hidden="true">{follow ? "[ON]" : "[OFF]"}</span>
            </Toggle>
            <Button size="sm" onClick={refresh}>
              Refresh
            </Button>
            {lines.length > 0 ? <CopyButton text={lines.join("\n")} label="copy lines" ariaLabel={`Copy the ${lines.length} visible log lines`} /> : null}
          </div>

          {error ? (
            <p role="alert" className="font-mono text-xs leading-relaxed text-destructive">
              Problem: {error}
              {tail ? " Showing the last tail that loaded." : ""}
            </p>
          ) : null}

          {noSource ? (
            <Empty
              kicker="no log"
              title={`${current.plain_name} has no log file on this node`}
              action={
                <ButtonLink size="sm" href={hrefTo(`/services/${current.id}`)}>
                  Open the service page
                </ButtonLink>
              }
            >
              The node reports no log source for this service (source: none), so there is nothing to follow. Its service page says whether it is running.
            </Empty>
          ) : (
            <MiniTerminal
              title={current.name}
              suffix="log"
              lines={lines}
              follow={follow}
              live={follow && !error}
              maxHeightClass="max-h-[60vh]"
              emptyText={emptyLogText(tail, loading)}
            />
          )}

          <p className="break-words font-mono text-[10px] text-muted-foreground">
            {tailSummary(tail)} · {followText(follow)}
            {at ? ` · updated ${new Date(at).toLocaleTimeString()}` : ""}
          </p>
        </section>
      )}
    </div>
  );
}
