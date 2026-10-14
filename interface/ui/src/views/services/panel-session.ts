// The Panel tab's session: mint a one-time ticket, point the frame at it, mint a new one on reload. Plain TypeScript with
// the network call injected, so "a reload asks the node for a fresh ticket" is tested without a browser.
//
// A ticket is single use and lives 30 s (os/messaging/README.md section 13), so a frame is never pointed at an old one:
// every start is a new generation, the newest answer wins, and an answer that arrives after a newer start (or after the
// tab closed) is dropped. The URL lives in this state and in the frame's `src`; nothing else holds it.
import type { PanelSession } from "../../lib/api/types";
import { checkMintReply, classifyMint, mintFailureText, PanelUrlError, type MintFailure } from "./panel-model";

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
  /** Every failed mint (the frame's and the full client's), before it becomes state (the view sends a 401 to the pairing page). */
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
      // The reply is data: no URL, or one the page would not frame, is a refusal, and a missing or absurd `expires_in` is the
      // documented 30 s (or at most 300 s), so the "spent ticket" note is neither immediate nor never.
      const reply = checkMintReply(session);
      const src = reply ? this.deps.src(reply.url) : null;
      if (reply === null || src === null) {
        this.set({ kind: "failed", gen, why: "bad_url", text: mintFailureText("bad_url") });
        return;
      }
      this.set({ kind: "ready", gen, src, expiresIn: reply.expiresIn });
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
   * Rejects when the node refuses (after telling `onError`, so a 401 reaches the pairing page here too) or answers an
   * address the page would not open (a `PanelUrlError`).
   */
  async fresh(): Promise<string> {
    let session: PanelSession;
    try {
      session = await this.deps.mint();
    } catch (err) {
      this.deps.onError?.(err);
      throw err;
    }
    const reply = checkMintReply(session);
    const src = reply ? this.deps.src(reply.url) : null;
    if (src === null) throw new PanelUrlError();
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
