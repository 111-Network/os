// Add a contact (os/5-messaging/README.md section 7.2): paste a card or a node1... id, scan a QR with the camera
// (secure origins only) or read one from a picture. A scan fills the box and stops there: the person looks, then adds.
import { useId, useRef, useState, type FormEvent } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo } from "../../app/location";
import { Button, ButtonLink } from "../../components/button";
import { Panel } from "../../components/panel";
import { Textarea } from "../../components/textarea";
import { apiStore, getAdapter } from "../../lib/api";
import { classifyAddInput, createBody } from "../../lib/api/contacts";
import type { Contact } from "../../lib/api/types";
import { addedSentence, addFailureSentence, addPrior, understood } from "./model";
import { cameraSupport, scanFile } from "./scan";
import { Scanner } from "./scanner";

type Message = { kind: "ok" | "error"; text: string; contact?: Contact };

export function AddContact() {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [message, setMessage] = useState<Message | null>(null);
  const file = useRef<HTMLInputElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);
  const hintId = useId();

  const here = typeof window === "undefined" ? null : window.location;
  const support = cameraSupport(
    { protocol: here?.protocol ?? "http:", hostname: here?.hostname ?? "" },
    typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia,
    typeof window === "undefined" ? undefined : window.isSecureContext,
  );
  const hint = understood(text);

  const got = (scanned: string) => {
    setText(scanned);
    setScanning(false);
    setMessage({ kind: "ok", text: "Read a code. Check it below, then press Add contact." });
    box.current?.focus();
  };

  const onFile = async (f: File | undefined) => {
    if (!f) return;
    const r = await scanFile(f);
    if (r.ok) got(r.text);
    else setMessage({ kind: "error", text: r.problem });
    if (file.current) file.current.value = "";
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    const body = createBody(classifyAddInput(text));
    if (!body) {
      setMessage({ kind: "error", text: understood(text).text });
      return;
    }
    setBusy(true);
    setMessage(null);
    try {
      const adapter = await getAdapter();
      const before = apiStore.getState().contacts;
      const contact = await adapter.contacts.add(body);
      setText("");
      setMessage({ kind: "ok", text: addedSentence(contact, addPrior(before, contact.node_id)), contact });
      await Promise.all([apiStore.refreshContacts(), apiStore.refreshThreads()]);
    } catch (err) {
      if (redirectIfUnauthenticated(err)) return;
      setMessage({ kind: "error", text: addFailureSentence(err) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Panel title="add a contact" suffix="card or id" heading={2}>
      <form className="flex flex-col gap-3 font-mono" onSubmit={(e) => void submit(e)}>
        <label htmlFor="add-contact-text" className="text-[10px] uppercase tracking-widest text-muted-foreground">
          Card text or node1… id
        </label>
        <Textarea
          id="add-contact-text"
          ref={box}
          rows={3}
          value={text}
          autoComplete="off"
          spellCheck={false}
          aria-describedby={hintId}
          placeholder="Paste their card, or their node1… id"
          className="break-all"
          onChange={(e) => {
            setText(e.target.value);
            setMessage(null);
          }}
        />
        <p id={hintId} className={hint.kind === "problem" ? "text-xs leading-snug text-destructive" : "text-xs leading-snug text-muted-foreground"}>
          {hint.kind === "none" ? "A card adds them with their keys and works with no connection. A node id alone asks your home hub for their card." : hint.text}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" variant="primary" loading={busy} aria-disabled={hint.kind !== "ok" || undefined}>
            Add contact
          </Button>
          {support.ok ? (
            <Button size="sm" aria-expanded={scanning} onClick={() => setScanning((s) => !s)}>
              {scanning ? "stop scanning" : "scan with the camera"}
            </Button>
          ) : null}
          <Button size="sm" onClick={() => file.current?.click()}>
            choose a QR picture
          </Button>
          <input ref={file} type="file" accept="image/*" className="sr-only" tabIndex={-1} aria-label="Choose a picture of a QR code" onChange={(e) => void onFile(e.target.files?.[0])} />
        </div>
        {!support.ok ? <p className="text-xs leading-snug text-muted-foreground">{support.text}</p> : null}
        {scanning && support.ok ? <Scanner onText={got} onClose={() => setScanning(false)} /> : null}
        <p role="status" className={message?.kind === "ok" ? "text-xs leading-snug text-foreground" : "sr-only"}>
          {message?.kind === "ok" ? message.text : ""}
        </p>
        <p role="alert" className={message?.kind === "error" ? "text-xs leading-snug text-destructive" : "sr-only"}>
          {message?.kind === "error" ? message.text : ""}
        </p>
        {message?.contact ? (
          <div>
            <ButtonLink size="sm" href={hrefTo(`/messages/${message.contact.node_id}`)}>
              {message.contact.request ? "Read the request" : "Write to them"}
            </ButtonLink>
          </div>
        ) : null}
      </form>
    </Panel>
  );
}
