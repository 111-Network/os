// The one plain-language sentence that leads Overview (spec 6.2). Pure: no React, no clock.
//
// Rules, first match wins:
//   1. No status yet                      -> "Checking your node..."            (attention)
//   2. Something is in error              -> "Your node is running, but <what> has a problem."  (error)
//        "what" = every service in `error`, the mesh transport when `transport_state` is
//        "transport degraded", and devices in `error`. A failing health check with nothing
//        nameable says so. This tone is never softened and never reads "OK".
//   3. The mesh transport is `starting`   -> "...The mesh transport is still starting."  (attention)
//   4. transport_state:
//        joined       -> "Your node is running and connected to nearby nodes - messages can flow."  (ok)
//        isolated     -> "Your node is running. It is alone on the mesh right now - no nearby nodes heard."  (attention)
//        no transport -> "Your node is running. It has no mesh transport yet - ask the agent to set one up."  (attention)
//   5. " It can reach the internet." is appended when the gateway service is live (any non-loading rule).
//
// The tone drives only the status dot next to the sentence; the sentence itself stays monochrome.
import type { DotState } from "../components/status-dot";
import type { DeviceRecord, ServiceRecord, Status } from "./api/types";

export type VerdictTone = "ok" | "attention" | "error";

export interface Verdict {
  sentence: string;
  tone: VerdictTone;
}

/** How each service reads inside a sentence. Falls back to the registry's plain name for ids from a newer daemon. */
const NOUN: Record<string, string> = {
  agent: "local AI agent",
  model: "local model",
  transport: "mesh transport",
  radio: "LoRa radio",
  kb: "knowledge base",
  gateway: "internet gateway",
  codeagent: "code agent",
  // Layer 5: the `fabric` row only goes to ERROR for a store that failed to open or is full.
  fabric: "message store",
  meshtastic: "Meshtastic board",
};

const noun = (s: Pick<ServiceRecord, "id" | "plain_name">): string => NOUN[s.id] ?? s.plain_name;

function joinPhrases(parts: readonly string[]): string {
  if (parts.length <= 1) return parts[0] ?? "";
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

function problemSentence(parts: readonly string[], total: number): string {
  if (parts.length > 3) return `Your node is running, but ${total} things have problems.`;
  return `Your node is running, but ${joinPhrases(parts)} ${total === 1 ? "has a problem" : "have problems"}.`;
}

export function verdict(
  status: Status | null,
  services: readonly ServiceRecord[] | null,
  devices: readonly DeviceRecord[] | null,
): Verdict {
  if (!status) return { sentence: "Checking your node…", tone: "attention" };
  const list = services ?? [];
  const internet = list.some((s) => s.id === "gateway" && s.state === "live") ? " It can reach the internet." : "";

  const parts = list.filter((s) => s.state === "error").map((s) => `the ${noun(s)}`);
  let things = parts.length; // "2 connected devices" is one phrase but two things
  const transportListedAsError = list.some((s) => s.id === "transport" && s.state === "error");
  if (status.transport_state === "transport degraded" && !transportListedAsError) {
    parts.unshift("the mesh transport");
    things += 1;
  }
  const badDevices = (devices ?? []).filter((d) => d.state === "error").length;
  if (badDevices > 0) {
    parts.push(badDevices === 1 ? "a connected device" : `${badDevices} connected devices`);
    things += badDevices;
  }

  if (parts.length > 0) return { sentence: problemSentence(parts, things) + internet, tone: "error" };
  if (status.health.status === "error") {
    return { sentence: `Your node is running, but its health check reports a problem.${internet}`, tone: "error" };
  }

  if (list.some((s) => s.id === "transport" && s.state === "starting") && status.transport_state !== "joined") {
    return { sentence: `Your node is running. The mesh transport is still starting.${internet}`, tone: "attention" };
  }

  switch (status.transport_state) {
    case "joined":
      return { sentence: `Your node is running and connected to nearby nodes — messages can flow.${internet}`, tone: "ok" };
    case "isolated":
      return { sentence: `Your node is running. It is alone on the mesh right now — no nearby nodes heard.${internet}`, tone: "attention" };
    case "no transport":
      return { sentence: `Your node is running. It has no mesh transport yet — ask the agent to set one up.${internet}`, tone: "attention" };
    default:
      return { sentence: `Your node is running.${internet}`, tone: "attention" };
  }
}

/** The dot beside the verdict. Hollow while disconnected: the last known state is no longer proof. */
export function verdictDot(tone: VerdictTone, disconnected: boolean): DotState {
  if (disconnected) return "unknown";
  return tone === "ok" ? "live" : tone === "error" ? "error" : "unset";
}
