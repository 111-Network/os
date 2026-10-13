// Settings > Essentials (spec section 6.7): five rows, then Sharing. Reachable in both modes.
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { announce } from "../../app/LiveRegion";
import { Button } from "../../components/button";
import { CopyButton } from "../../components/copy-button";
import { Input } from "../../components/input";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { Segmented } from "../../components/segmented";
import { SpecTable } from "../../components/spec-table";
import { Toggle } from "../../components/toggle";
import { apiStore, getAdapter, type Adapter } from "../../lib/api";
import { useConnection, useStatus } from "../../lib/api/hooks";
import type { Settings } from "../../lib/api/types";
import { useMode, type Mode } from "../../lib/mode";
import { toSpecRows } from "./facts";
import {
  BACKUP_COMMAND,
  LOOPBACK_ONLY_TEXT,
  aliasPatchValue,
  essentialRows,
  isLoopbackOnlyError,
  isSettingsLockedError,
  lanNote,
  offerActions,
  offerLabel,
  problemText,
  restartConfirmText,
  sharingDisabled,
  updateRows,
  type EssentialId,
} from "./model";
import { Feedback, Row, FEEDBACK } from "./row";

// ---------------------------------------------------------------- shared hooks

interface ActionState {
  pending: boolean;
  error: string | null;
  loopback: boolean;
  /** 409 settings_locked: the node's environment file pins the setting. */
  locked: boolean;
}

const IDLE: ActionState = { pending: false, error: null, loopback: false, locked: false };

/**
 * Runs one node action at a time and keeps its pending/problem state. A 401 sends the browser to
 * /pair; a 403 loopback_only is reported as `loopback` so the control can show the disabled text,
 * and a 409 settings_locked as `locked` for the pinned note.
 */
function useAction(): [ActionState, <T>(fn: (a: Adapter) => Promise<T>) => Promise<T | undefined>] {
  const [state, setState] = useState<ActionState>(IDLE);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const run = useCallback(async <T,>(fn: (a: Adapter) => Promise<T>): Promise<T | undefined> => {
    setState({ pending: true, error: null, loopback: false, locked: false });
    try {
      const result = await fn(await getAdapter());
      if (alive.current) setState(IDLE);
      return result;
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return undefined;
      if (alive.current) setState({ pending: false, error: problemText(err), loopback: isLoopbackOnlyError(err), locked: isSettingsLockedError(err) });
      return undefined;
    }
  }, []);
  return [state, run];
}

/** `GET /settings` once on mount. Failures are kept so the page can say so; 401 goes to /pair. */
function useSettings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    getAdapter()
      .then((a) => a.settings())
      .then((s) => {
        if (alive) setSettings(s);
      })
      .catch((err: unknown) => {
        if (alive && !redirectIfUnauthenticated(err)) setError(problemText(err));
      });
    return () => {
      alive = false;
    };
  }, []);
  return { settings, setSettings, error };
}

// ---------------------------------------------------------------- rows

function AliasRow({ settings, onSaved }: { settings: Settings | null; onSaved: (s: Settings) => void }) {
  const status = useStatus();
  const [draft, setDraft] = useState<string | null>(null);
  const [action, run] = useAction();
  const [saved, setSaved] = useState(false);
  const baseline = settings?.alias ?? status?.node.alias ?? "";
  const value = draft ?? baseline;
  const dirty = draft !== null && aliasPatchValue(draft) !== (baseline === "" ? null : baseline);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (draft === null) return;
    setSaved(false);
    const next = await run((a) => a.patchSettings({ alias: aliasPatchValue(draft) }));
    if (!next) return;
    onSaved(next);
    setDraft(null);
    setSaved(true);
    announce("Node name saved");
    void apiStore.refreshStatus();
  };

  return (
    <form onSubmit={(e) => void submit(e)} noValidate>
      <div className="flex flex-col gap-2 sm:flex-row">
        <Input
          value={value}
          maxLength={64}
          autoComplete="off"
          spellCheck={false}
          aria-labelledby="alias-label"
          placeholder={status?.node.label ?? "Name this node"}
          onChange={(e) => {
            setSaved(false);
            setDraft(e.target.value);
          }}
        />
        <Button type="submit" variant="primary" arrow={false} disabled={!dirty} loading={action.pending}>
          Save
        </Button>
      </div>
      <Feedback text={action.error ?? (saved ? "Saved." : null)} problem={action.error !== null} />
    </form>
  );
}

function ModeRow() {
  const { mode, setMode } = useMode();
  return (
    <Segmented<Mode>
      label="View mode"
      value={mode}
      onChange={setMode}
      options={[
        { value: "simple", label: "SIMPLE" },
        { value: "advanced", label: "ADVANCED" },
      ]}
    />
  );
}

function BackupRow({ note }: { note: string }) {
  return (
    <div className="flex flex-col gap-3 border border-border bg-secondary/20 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <code className="min-w-0 break-all border border-border bg-background px-3 py-2 font-mono text-xs text-foreground">
          {BACKUP_COMMAND}
        </code>
        <CopyButton text={BACKUP_COMMAND} ariaLabel="Copy the identity backup command" />
      </div>
      <p className={`text-muted-foreground ${FEEDBACK}`}>{note}</p>
    </div>
  );
}

function RestartRow() {
  const status = useStatus();
  const conn = useConnection();
  const [action, run] = useAction();
  // null = not requested; otherwise the uptime seen when the request went out.
  const [requestedAt, setRequestedAt] = useState<{ uptime: number | null } | null>(null);
  const [back, setBack] = useState(false);
  const sawDrop = useRef(false);

  useEffect(() => {
    if (!requestedAt || back) return;
    if (conn.disconnected) sawDrop.current = true;
    const uptime = status?.node.uptime_sec ?? null;
    const reset = requestedAt.uptime !== null && uptime !== null && uptime < requestedAt.uptime;
    if ((sawDrop.current && !conn.disconnected) || reset) {
      setBack(true);
      announce("The node restarted and the page reconnected.");
    }
  }, [requestedAt, back, conn.disconnected, status?.node.uptime_sec]);

  const restart = async () => {
    const uptime = status?.node.uptime_sec ?? null;
    const reply = await run((a) => a.restart());
    if (!reply) return;
    sawDrop.current = false;
    setBack(false);
    setRequestedAt({ uptime });
    announce("Restarting. This page will reconnect.");
  };

  const waiting = requestedAt !== null && !back;
  return (
    <div>
      <Button variant="destructive" confirm={restartConfirmText(status)} loading={action.pending} disabled={waiting || status === null} onClick={() => void restart()}>
        Restart node
      </Button>
      {waiting ? (
        <p role="status" className={`mt-2 text-foreground ${FEEDBACK}`}>
          restarting — this page will reconnect
        </p>
      ) : back ? (
        <p role="status" className={`mt-2 text-muted-foreground ${FEEDBACK}`}>
          The node restarted and this page reconnected.
        </p>
      ) : null}
      {action.error ? <Feedback text={action.error} problem /> : null}
    </div>
  );
}

function UpdateRow() {
  const status = useStatus();
  const [action, run] = useAction();
  const [note, setNote] = useState<string | null>(null);
  const checking = status?.update.state === "checking";

  const check = async () => {
    setNote(null);
    const reply = await run((a) => a.updateCheck());
    if (!reply) return;
    const text = reply.started ? "Check started. The result appears below." : "A check is already in progress.";
    setNote(text);
    announce(text);
    void apiStore.refreshStatus();
  };

  return (
    <div className="flex flex-col gap-3">
      <div>
        <Button loading={action.pending || checking} disabled={status === null} onClick={() => void check()}>
          Check now
        </Button>
        <Feedback text={action.error ?? note} problem={action.error !== null} />
      </div>
      <SpecTable rows={toSpecRows(updateRows(status))} />
    </div>
  );
}

// ---------------------------------------------------------------- sharing

function Gate({ id }: { id: string }) {
  return (
    <p id={id} className={`mt-2 text-muted-foreground ${FEEDBACK}`}>
      {LOOPBACK_ONLY_TEXT}
    </p>
  );
}

function SharingPanel({ settings, onSaved }: { settings: Settings | null; onSaved: (s: Settings) => void }) {
  const status = useStatus();
  const [lanAction, runLan] = useAction();
  const [offerAction, runOffer] = useAction();
  // A 409 settings_locked arrived before the settings were refetched.
  const [lockedSeen, setLockedSeen] = useState(false);

  useEffect(() => {
    if (!lanAction.locked) return;
    setLockedSeen(true);
    // Show what the node actually runs with (the pinned value), not what was asked for.
    void getAdapter()
      .then((a) => a.settings())
      .then(onSaved)
      .catch(() => undefined);
  }, [lanAction.locked, onSaved]);

  const gated = sharingDisabled(status) || lanAction.loopback || offerAction.loopback;
  const lanOn = settings?.lan_enabled ?? status?.lan_enabled ?? false;
  const known = settings !== null || status !== null;
  const offer = settings ? settings.offer : (status?.offer?.state ?? null);
  const actions = offerActions(offer);
  const note = lanNote(status, settings, lockedSeen);
  const locked = note?.kind === "locked";

  const toggleLan = async (next: boolean) => {
    const result = await runLan((a) => a.patchSettings({ lan_enabled: next }));
    if (!result) return;
    onSaved(result);
    // What the node answered, not what was asked: a pinned value comes back unchanged.
    announce(lanNote(status, result)?.text ?? `LAN sharing ${result.lan_enabled ? "on" : "off"}.`);
    void apiStore.refreshStatus();
  };

  const decide = async (choice: "accepted" | "dismissed") => {
    const result = await runOffer((a) => a.patchSettings({ offer: choice }));
    if (!result) return;
    onSaved(result);
    announce(`Standard tier ${choice}.`);
    void apiStore.refreshStatus();
  };

  return (
    <Panel title="sharing" flush>
      <Row
        id="lan"
        label="LAN sharing"
        help={
          <>
            LAN visitors must enter the pair code shown by <code className="text-foreground">111 dashboard</code> on the node. Traffic is
            unencrypted on shared Wi-Fi. Takes effect after a restart.
          </>
        }
      >
        <Toggle
          pressed={lanOn}
          onPressedChange={(next) => void toggleLan(next)}
          disabled={gated || locked || !known || lanAction.pending}
          aria-describedby={gated ? "lan-gate" : locked ? "lan-note" : undefined}
        >
          <span>LAN sharing</span> <span aria-hidden="true">{lanOn ? "[ON]" : "[OFF]"}</span>
        </Toggle>
        {gated ? <Gate id="lan-gate" /> : null}
        {!gated && note ? (
          <p id="lan-note" role="status" className={`mt-2 text-muted-foreground ${FEEDBACK}`}>
            {note.text}
          </p>
        ) : null}
        {!gated && !lanAction.locked ? <Feedback text={lanAction.error} problem /> : null}
      </Row>
      <Row
        id="offer"
        label="Standard tier (local model)"
        help="A small AI model that runs on this node, so the agent can answer without the internet. Accepting records your choice on the node."
      >
        <p className={`text-foreground ${FEEDBACK}`}>
          <span className="text-muted-foreground">state: </span>
          {offerLabel(offer)}
        </p>
        <div className={actions.accept || actions.dismiss ? "mt-3 flex flex-wrap gap-2" : undefined}>
          {actions.accept ? (
            <Button variant="primary" arrow={false} size="sm" disabled={gated || offerAction.pending} onClick={() => void decide("accepted")} aria-describedby={gated ? "offer-gate" : undefined}>
              Accept
            </Button>
          ) : null}
          {actions.dismiss ? (
            <Button size="sm" disabled={gated || offerAction.pending} onClick={() => void decide("dismissed")} aria-describedby={gated ? "offer-gate" : undefined}>
              Dismiss
            </Button>
          ) : null}
        </div>
        {gated ? <Gate id="offer-gate" /> : null}
        {!gated ? <Feedback text={offerAction.error} problem /> : null}
      </Row>
    </Panel>
  );
}

// ---------------------------------------------------------------- page

export function Essentials() {
  const { settings, setSettings, error } = useSettings();
  const rows = essentialRows();

  const control = (id: EssentialId, note?: string) => {
    switch (id) {
      case "alias":
        return <AliasRow settings={settings} onSaved={setSettings} />;
      case "mode":
        return <ModeRow />;
      case "identity-backup":
        return <BackupRow note={note ?? ""} />;
      case "restart":
        return <RestartRow />;
      case "update-check":
        return <UpdateRow />;
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader level={1} kicker="settings" title="Essentials" />
      {error ? (
        <p role="alert" className={`text-destructive ${FEEDBACK}`}>
          Problem: settings could not be loaded. {error}
        </p>
      ) : null}
      <Panel title="essentials" flush>
        {rows.map((r) => (
          <Row key={r.id} id={r.id} label={r.label} help={r.help}>
            {control(r.id, r.note)}
          </Row>
        ))}
      </Panel>
      <SharingPanel settings={settings} onSaved={setSettings} />
    </div>
  );
}
