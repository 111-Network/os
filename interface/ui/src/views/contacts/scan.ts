// QR scanning for "add a contact" (os/messaging/README.md sections 7.2 and 16). The decoder is jsQR, loaded on the
// first scan only (it is a separate chunk), and nothing leaves the browser: pixels go in, a string comes out.
//
// Two inputs: a camera, which needs a secure context (HTTPS or the node's own loopback origin, so a node opened over a
// plain-HTTP LAN address never asks for the camera), and a picture file, which works everywhere.
import { isSecureOrigin, type PanelLocation } from "../../lib/api/panels";

/** Decodes RGBA pixels to the QR payload, or null when there is no readable code. */
export type QrDecoder = (rgba: Uint8ClampedArray, width: number, height: number) => string | null;

/** The frame is scaled to at most this many pixels on its longest side before decoding (jsQR is linear in pixels). */
export const MAX_SCAN_SIDE = 1024;
/** A picture larger than this is refused before it is decoded. */
export const MAX_SCAN_FILE_BYTES = 12_000_000;
/** The camera is read at most this often. */
export const SCAN_INTERVAL_MS = 120;

let decoderPromise: Promise<QrDecoder> | null = null;

/** The jsQR decoder, loaded once. A failed load is not cached, so the next scan tries again. */
export function loadDecoder(): Promise<QrDecoder> {
  decoderPromise ??= import("jsqr").then(
    (m) => {
      const jsQR = m.default;
      return (rgba, width, height) => jsQR(rgba, width, height, { inversionAttempts: "attemptBoth" })?.data ?? null;
    },
    (e: unknown) => {
      decoderPromise = null;
      throw e;
    },
  );
  return decoderPromise;
}

export type CameraSupport = { ok: true } | { ok: false; reason: "insecure" | "no_camera"; text: string };

/** Camera scanning is offered on HTTPS and loopback origins only, and only when the browser has a camera API. */
export function cameraSupport(loc: PanelLocation, hasCameraApi: boolean, secureContext: boolean | undefined = undefined): CameraSupport {
  if (!isSecureOrigin(loc) || secureContext === false) {
    return {
      ok: false,
      reason: "insecure",
      text: "The camera only works on a secure page: open the dashboard on the node itself, or over HTTPS. You can still choose a picture of the code.",
    };
  }
  if (!hasCameraApi) return { ok: false, reason: "no_camera", text: "This browser cannot reach a camera. You can still choose a picture of the code." };
  return { ok: true };
}

/** Plain words for a getUserMedia failure. */
export function cameraErrorText(name: string | undefined): string {
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
      return "The camera was not allowed. Allow it for this page, or choose a picture of the code instead.";
    case "NotFoundError":
    case "OverconstrainedError":
      return "No camera was found on this device. Choose a picture of the code instead.";
    case "NotReadableError":
      return "The camera is in use by something else. Close the other app, or choose a picture of the code instead.";
    default:
      return "The camera could not be started. Choose a picture of the code instead.";
  }
}

/** Largest scale (at most 1) that fits `width` x `height` within `max` on the longest side. */
export function fitScale(width: number, height: number, max: number = MAX_SCAN_SIDE): number {
  const longest = Math.max(width, height);
  return longest > max ? max / longest : 1;
}

export type ScanResult = { ok: true; text: string } | { ok: false; problem: string };

/** Decode one frame of pixels. Never throws: a broken decoder reads as "no code found". */
export function decodeFrame(decode: QrDecoder, rgba: Uint8ClampedArray, width: number, height: number): string | null {
  if (!(width > 0) || !(height > 0) || rgba.length !== width * height * 4) return null;
  try {
    return decode(rgba, width, height);
  } catch {
    return null;
  }
}

/** Read a QR code out of a picture file. */
export async function scanFile(file: File, decode?: QrDecoder): Promise<ScanResult> {
  if (file.size > MAX_SCAN_FILE_BYTES) return { ok: false, problem: "That picture is too large. Use a smaller one." };
  if (file.type !== "" && !file.type.startsWith("image/")) return { ok: false, problem: "That file is not a picture." };
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    return { ok: false, problem: "That picture could not be opened." };
  }
  try {
    const scale = fitScale(bitmap.width, bitmap.height);
    const w = Math.max(1, Math.round(bitmap.width * scale));
    const h = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return { ok: false, problem: "This browser cannot read pictures here." };
    ctx.drawImage(bitmap, 0, 0, w, h);
    const decoder = decode ?? (await loadDecoder());
    const text = decodeFrame(decoder, ctx.getImageData(0, 0, w, h).data, w, h);
    return text === null || text.trim() === "" ? { ok: false, problem: "No QR code was found in that picture. Try a sharper, closer one." } : { ok: true, text };
  } catch {
    return { ok: false, problem: "The QR reader could not be loaded. Paste the card text instead." };
  } finally {
    bitmap.close();
  }
}
