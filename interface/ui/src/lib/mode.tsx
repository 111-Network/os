// Interface density mode (spec section 5): simple | advanced. Density only: never a node write and
// never a security boundary. Resolution: `?mode=` (consumed once, then stripped from the URL) >
// localStorage `111.ui.mode` > simple. The toggle always wins afterwards and is persisted.
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import type { UiMode } from "../components/spec-table";
import { consumeParam } from "./params";

export type Mode = UiMode;

export const MODE_STORAGE_KEY = "111.ui.mode";

function isMode(v: unknown): v is Mode {
  return v === "simple" || v === "advanced";
}

function parseMode(v: string | null | undefined): Mode | null {
  const s = v?.trim().toLowerCase();
  return isMode(s) ? s : null;
}

/** Pure precedence: a valid URL value beats a valid stored value beats "simple". Junk is ignored. */
export function resolveMode(urlValue: string | null | undefined, storedValue: string | null | undefined): Mode {
  return parseMode(urlValue) ?? parseMode(storedValue) ?? "simple";
}

function readStored(): string | null {
  try {
    return window.localStorage.getItem(MODE_STORAGE_KEY);
  } catch {
    return null; // storage blocked (private mode, disabled cookies)
  }
}

function writeStored(mode: Mode): void {
  try {
    window.localStorage.setItem(MODE_STORAGE_KEY, mode);
  } catch {
    /* the mode still works for this page load */
  }
}

// The URL parameter is stripped the first time it is read, and React StrictMode runs state
// initializers twice, so the answer is computed once per page load.
let initial: Mode | null = null;

export function initialMode(): Mode {
  if (initial) return initial;
  if (typeof window === "undefined") return "simple";
  const fromUrl = consumeParam("mode");
  initial = resolveMode(fromUrl, readStored());
  if (parseMode(fromUrl)) writeStored(initial);
  return initial;
}

/** Test seam. */
export function resetModeForTests(): void {
  initial = null;
}

export interface ModeValue {
  mode: Mode;
  setMode: (mode: Mode) => void;
}

const ModeContext = createContext<ModeValue>({ mode: "simple", setMode: () => {} });

export function ModeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<Mode>(initialMode);
  const setMode = useCallback((next: Mode) => {
    setModeState(next);
    writeStored(next);
  }, []);
  const value = useMemo<ModeValue>(() => ({ mode, setMode }), [mode, setMode]);
  return <ModeContext value={value}>{children}</ModeContext>;
}

export function useMode(): ModeValue {
  return useContext(ModeContext);
}

/** Renders its children in advanced mode only. */
export function Adv({ children }: { children?: ReactNode }) {
  return useMode().mode === "advanced" ? <>{children}</> : null;
}
