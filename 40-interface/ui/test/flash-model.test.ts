import { describe, expect, it } from "vitest";
import { ApiError } from "../src/lib/api/problem";
import { computeAttention } from "../src/lib/api/store";
import type { DeviceRecord, FlashOffer } from "../src/lib/api/types";
import {
  EMPTY_CONFIRM,
  FLASH_NOT_AVAILABLE,
  FLASH_SOURCE,
  FLASH_STALE,
  FLASH_STALLED_AFTER_MS,
  NEEDS_FIRMWARE,
  confirmMissing,
  confirmValid,
  erasesSentence,
  familyName,
  flashButtonLabel,
  flashErrorText,
  flashRequest,
  flashStalled,
  hasFirmwareNow,
  missingText,
  nameMatches,
  needsFirmware,
  offerDisplay,
  shortSha,
  whySentence,
  type FlashConfirm,
} from "../src/views/devices/flash-model";

const SHA = "f87a2e8af657f07642eee8206b15f83e295c58528c92e497b5d3b5ec8c84ed9c";

const offer: FlashOffer = {
  reason: "blank",
  image: { family: "rnode", version: "1.78", file: "share/firmware/rnode-heltec-v3-1.78.bin", sha256: SHA, board: "Heltec WiFi LoRa 32 V3" },
  erases: "everything on the board, including any Meshtastic settings",
};

const board = {
  alias: null,
  name: "Heltec WiFi LoRa 32 V3",
  kind: "radio",
  transport: "usb-serial",
  match: { confidence: "likely", rule: "lora-table 303a:1001" },
} as Pick<DeviceRecord, "alias" | "name" | "kind" | "transport" | "match">;

const problem = (status: number, code: string, detail?: string) =>
  new ApiError({ type: "about:blank", title: "Problem", status, code: code as never, ...(detail ? { detail } : {}) });

const ticked = (typed: string): FlashConfirm => ({ checks: { erase: true, own: true }, typed });

describe("offerDisplay", () => {
  const d = offerDisplay(board, offer);

  it("lists exactly what would be written", () => {
    expect(d.lines).toEqual([
      { label: "Firmware", value: "RNode 1.78" },
      { label: "Built for", value: "Heltec WiFi LoRa 32 V3" },
      { label: "Image file", value: "share/firmware/rnode-heltec-v3-1.78.bin" },
      { label: "SHA-256", value: "f87a2e8a…8c84ed9c" },
    ]);
  });
  it("shortens the sha256 but keeps the full value for the disclosure", () => {
    expect(d.sha).toEqual({ short: "f87a2e8a…8c84ed9c", full: SHA });
    expect(d.lines[3]?.value).toBe(d.sha.short);
  });
  it("says where the image comes from, in plain words", () => {
    expect(d.source).toBe(FLASH_SOURCE);
    expect(d.source).toMatch(/signed release/);
    expect(d.source).toMatch(/Nothing is downloaded/);
  });
  it("turns erases into the warning sentence", () => {
    expect(d.erases).toBe("Flashing erases everything on the board, including any Meshtastic settings.");
  });
  it("names the board and how it is attached", () => {
    expect(d.board).toBe("Heltec WiFi LoRa 32 V3 · LoRa board · USB serial");
    expect(offerDisplay({ ...board, alias: "Bench radio" }, offer).board).toBe("Bench radio (detected as Heltec WiFi LoRa 32 V3) · LoRa board · USB serial");
  });
  it("reads differently for blank and unknown", () => {
    expect(whySentence("blank")).toMatch(/looks blank/);
    expect(whySentence("unknown")).toMatch(/Something answered/);
    expect(whySentence("blank")).not.toBe(whySentence("unknown"));
    expect(offerDisplay(board, { ...offer, reason: "unknown" }).why).toBe(whySentence("unknown"));
    expect(whySentence("something-new")).toBe(whySentence("unknown")); // a reason this UI does not know is not called blank
  });
  it("asks the user to check the board when it was matched by USB ids only", () => {
    expect(d.check).toMatch(/USB ids, which name the chip, not the board/);
    expect(d.check).toContain("Heltec WiFi LoRa 32 V3");
    expect(offerDisplay({ ...board, match: { confidence: "unknown", rule: "none" } }, offer).check).not.toBeNull();
    expect(offerDisplay({ ...board, match: { confidence: "exact", rule: "probe" } }, offer).check).toBeNull();
  });
  it("cleans untrusted strings and bounds them", () => {
    const evil: FlashOffer = { ...offer, image: { ...offer.image, board: "Board\u0007‮x", file: "f".repeat(500), version: "1.78\n" }, erases: "all\u0000 of it.." };
    const e = offerDisplay(board, evil);
    expect(e.lines[1]?.value).toBe("Boardx");
    expect(e.lines[2]?.value).toHaveLength(200);
    expect(e.lines[0]?.value).toBe("RNode 1.78");
    expect(e.erases).toBe("Flashing erases all of it.");
  });
});

describe("shortSha and erasesSentence", () => {
  it("keeps 8 + 8 of a 64-hex digest, anything else is cut to fit", () => {
    expect(shortSha(SHA)).toBe("f87a2e8a…8c84ed9c");
    expect(shortSha("abc")).toBe("abc");
    expect(shortSha("z".repeat(40))).toBe(`${"z".repeat(16)}…`);
    expect(shortSha("")).toBe("");
  });
  it("builds one sentence around the phrase and never doubles the full stop", () => {
    expect(erasesSentence("everything on the board")).toBe("Flashing erases everything on the board.");
    expect(erasesSentence("everything on the board.  ")).toBe("Flashing erases everything on the board.");
  });
  it("falls back to the broadest true statement when the node says nothing", () => {
    for (const none of ["", "   ", undefined, null]) expect(erasesSentence(none)).toBe("Flashing erases everything that is on the board now.");
  });
});

describe("naming and labels", () => {
  it("names the family and builds the one primary button", () => {
    expect(familyName("rnode")).toBe("RNode");
    expect(flashButtonLabel(offer)).toBe("Flash RNode firmware…");
    expect(familyName("other\u0007")).toBe("other");
  });
  it("tags a board only when the node offers it firmware", () => {
    expect(needsFirmware({ flash_offer: offer })).toBe(true);
    expect(needsFirmware({ flash_offer: null })).toBe(false);
    expect(needsFirmware({})).toBe(false);
    expect(NEEDS_FIRMWARE).toBe("NEEDS FIRMWARE");
  });
  it("hasFirmwareNow is true only for a family the node can use", () => {
    expect(hasFirmwareNow({ firmware: { family: "rnode", version: "1.78", source: "probe" } })).toBe(true);
    expect(hasFirmwareNow({ firmware: { family: "none", version: null, source: "probe" } })).toBe(false);
    expect(hasFirmwareNow({ firmware: { family: "unknown", version: null, source: "unknown" } })).toBe(false);
    expect(hasFirmwareNow({ firmware: null })).toBe(false);
  });
});

describe("nameMatches", () => {
  it("compares trimmed and case-sensitive", () => {
    expect(nameMatches("Bench radio", "Bench radio")).toBe(true);
    expect(nameMatches("  Bench radio  ", "Bench radio")).toBe(true);
    expect(nameMatches("Bench radio", " Bench radio ")).toBe(true);
    expect(nameMatches("bench radio", "Bench radio")).toBe(false);
    expect(nameMatches("Bench  radio", "Bench radio")).toBe(false);
    expect(nameMatches("Bench", "Bench radio")).toBe(false);
  });
  it("never matches an empty name, even against an empty field", () => {
    expect(nameMatches("", "")).toBe(false);
    expect(nameMatches("   ", "  ")).toBe(false);
    expect(nameMatches("x", "")).toBe(false);
  });
});

describe("confirmation", () => {
  const name = "Heltec WiFi LoRa 32 V3";

  it("needs both boxes and the typed name", () => {
    expect(confirmMissing(EMPTY_CONFIRM, name)).toEqual(["erase", "own", "name"]);
    expect(confirmMissing({ checks: { erase: true, own: false }, typed: name }, name)).toEqual(["own"]);
    expect(confirmMissing({ checks: { erase: false, own: true }, typed: name }, name)).toEqual(["erase"]);
    expect(confirmMissing({ checks: { erase: true, own: true }, typed: "heltec" }, name)).toEqual(["name"]);
    expect(confirmMissing(ticked(` ${name} `), name)).toEqual([]);
  });
  it("is valid only when nothing is missing", () => {
    expect(confirmValid(EMPTY_CONFIRM, name)).toBe(false);
    expect(confirmValid(ticked(name), name)).toBe(true);
    expect(confirmValid(ticked(name.toLowerCase()), name)).toBe(false);
    expect(confirmValid(ticked(name), "")).toBe(false);
  });
  it("says what is still needed, in one sentence", () => {
    expect(missingText([])).toBe("");
    expect(missingText(["erase", "own", "name"])).toBe("Still needed: tick both boxes and type the name exactly as shown.");
    expect(missingText(["own"])).toBe("Still needed: tick the second box.");
    expect(missingText(["erase", "name"])).toBe("Still needed: tick the first box and type the name exactly as shown.");
    expect(missingText(["name"])).toBe("Still needed: type the name exactly as shown.");
  });
  it("builds the request from the offer on screen: its sha256 and the trimmed typed name", () => {
    expect(flashRequest(offer, ticked("  Bench radio "))).toEqual({ sha256: SHA, confirm: "Bench radio" });
  });
});

describe("flashErrorText", () => {
  it("404 means the route is not there yet and the offer stays", () => {
    expect(flashErrorText(problem(404, "not_found"))).toBe(FLASH_NOT_AVAILABLE);
    expect(FLASH_NOT_AVAILABLE).toBe("Flashing is not available on this node yet (the flashing core lands in Layer 6). The offer stays here.");
  });
  it("409 offer_stale asks for a reload", () => {
    expect(flashErrorText(problem(409, "offer_stale", "sha256 differs"))).toBe(FLASH_STALE);
    expect(FLASH_STALE).toBe("The offer changed; reload the device.");
  });
  it("409 for another reason says what the node said, or that a flash is running", () => {
    expect(flashErrorText(problem(409, "internal", "A flash is already running."))).toBe("A flash is already running.");
    expect(flashErrorText(problem(409, "internal"))).toBe("This board is already being flashed.");
  });
  it("400 names the confirmation; other problems pass the node's own words through", () => {
    expect(flashErrorText(problem(400, "bad_request"))).toMatch(/confirmation/);
    expect(flashErrorText(problem(403, "lan_read_only", "Read-only from the LAN."))).toBe("Read-only from the LAN.");
    expect(flashErrorText(problem(500, "internal"))).toBe("Problem");
  });
  it("a lost connection and a stray error are plain too", () => {
    expect(flashErrorText(ApiError.network(new TypeError("Failed to fetch")))).toMatch(/Could not reach the node/);
    expect(flashErrorText(new Error("boom"))).toBe("Could not start the flash. Try again.");
    expect(flashErrorText(undefined)).toBe("Could not start the flash. Try again.");
  });
});

describe("flashStalled", () => {
  it("is true once the node has been silent about the new firmware for long enough", () => {
    expect(flashStalled(1_000, 1_000 + FLASH_STALLED_AFTER_MS - 1)).toBe(false);
    expect(flashStalled(1_000, 1_000 + FLASH_STALLED_AFTER_MS)).toBe(true);
  });
});

describe("attention", () => {
  it("an offered board is not counted again: the attention badge counts errors and proposals only", () => {
    const offered = { id: "usb-1", state: "needs_setup", flash_offer: offer } as unknown as DeviceRecord;
    expect(computeAttention([], [], [offered])).toEqual({ count: 0, openProposals: 0, erroredServices: 0, erroredDevices: 0, openRequests: 0 });
  });
});
