// The Panel tab's session: mint a one-time ticket, point the frame at it, mint a new one on reload. Plain TypeScript with
// the network call injected, so "a reload asks the node for a fresh ticket" is tested without a browser.
//
// A ticket is single use and lives 30 s (os/messaging/README.md section 13), so a frame is never pointed at an old one:
// every start is a new generation, the newest answer wins, and an answer that arrives after a newer start (or after the
// tab closed) is dropped. The URL lives in this state and in the frame's `src`; nothing else holds it.
import type { PanelSession } from "../../lib/api/types";
import { classifyMint, mintFailureText, type MintFailure } from "./panel-model";

export type SessionState =
  | { kind: "idle" }
  | { kind: "minting"; gen: number }
  | { kind: "ready"; gen: number; src: string; expiresIn: number }
  | { kind: "failed"; gen: number; why: MintFailure; text: string };

export interface SessionDeps {
  /** `POST /panels/{id}/session`. */
  mint(): Promise<PanelSession>;
  /** The URL the frame may load for the node's answer, or null when it must not (see `ticketSrc`). */
  src(url: string): string | null;
  /** Every failed mint, before it becomes state (the view sends a 401 to the pairing page). */
  onError?(err: unknown): void;
}

/** The classification and the sentence for a failed mint. */
function failure(err: unknown): { why: MintFailure; text: string } {
  const why = classifyMint(err);
  return { why, text: mintFailureText(why, err instanceof Error ? err.message : undefined) };
}

export class PanelSessionController {
  private state: SessionState = { kind: "idle" };
  private gen = 0;
  private closed = false;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly deps: SessionDeps) {}

  getState = (): SessionState => this.state;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  private set(next: SessionState): void {
    this.state = next;
    for (const l of [...this.listeners]) l();
  }

  /**
   * Mint a ticket and point the frame at it. Calling it again (reload, "try again") mints another one: a new generation,
   * which also keys the frame so it is rebuilt rather than navigated. Resolves when this start has been answered.
   */
  async start(): Promise<void> {
    this.closed = false;
    const gen = ++this.gen;
    this.set({ kind: "minting", gen });
    try {
      const session = await this.deps.mint();
      if (this.closed || gen !== this.gen) return;
      const src = this.deps.src(session.url);
      if (src === null) {
        this.set({ kind: "failed", gen, why: "bad_url", text: mintFailureText("bad_url") });
        return;
      }
      this.set({ kind: "ready", gen, src, expiresIn: session.expires_in });
    } catch (err) {
      if (this.closed || gen !== this.gen) return;
      this.deps.onError?.(err);
      this.set({ kind: "failed", gen, ...failure(err) });
    }
  }

  /** The reload button: a new ticket, a rebuilt frame. */
  reload(): Promise<void> {
    return this.start();
  }

  /**
   * A separate ticket for a separate window ("Open full client"). It does not touch the frame and is not remembered.
   * Rejects when the node refuses or answers an address the page would not open.
   */
  async fresh(): Promise<string> {
    const session = await this.deps.mint();
    const src = this.deps.src(session.url);
    if (src === null) throw new Error(mintFailureText("bad_url"));
    return src;
  }

  /** The tab closed: an answer still on its way is dropped. */
  dispose(): void {
    this.closed = true;
    this.gen++;
  }
}

export type OpenResult = { kind: "opened" } | { kind: "blocked" } | { kind: "failed"; why: MintFailure; text: string };

/** The few `Window` members a new tab needs. */
export interface OpenedWindow {
  opener: unknown;
  close(): void;
  document?: { body?: { textContent: string | null } | null };
  location: { replace(url: string): void };
}

/**
 * "Open full client": a new tab with its own ticket. The window is opened first, inside the click, so a popup blocker
 * sees a user gesture; it is cut off from this page (`opener = null`) before the ticket is minted, and only then pointed
 * at the ticket URL. A refused mint closes the empty tab again.
 */
export async function openFullClient(mint: () => Promise<string>, open: () => OpenedWindow | null, say = "Opening the panel…"): Promise<OpenResult> {
  const win = open();
  if (!win) return { kind: "blocked" };
  try {
    win.opener = null;
  } catch {
    /* some browsers refuse; the window is still ours alone */
  }
  try {
    if (win.document?.body) win.document.body.textContent = say;
  } catch {
    /* cross-origin already: nothing to say in it */
  }
  try {
    win.location.replace(await mint());
    return { kind: "opened" };
  } catch (err) {
    try {
      win.close();
    } catch {
      /* already gone */
    }
    return { kind: "failed", ...failure(err) };
  }
}
