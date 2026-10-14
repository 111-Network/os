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
/**
 * A picture with more pixels than this is refused before it is decoded: a small file can declare billions of pixels (a
 * 12 MB PNG of 40000 x 40000 compresses well), and decoding allocates width x height x 4 bytes. 36 million is 6000 x 6000,
 * far above what a photo of a QR code needs and about the most a 12 MB JPEG holds.
 */
export const MAX_SCAN_PIXELS = 36_000_000;
/** How much of the file is read to find its size: a JPEG's size sits after its metadata segments. */
export const IMAGE_HEAD_BYTES = 1 << 20;
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

// --- Picture size from the file header (read before anything is decoded) -----------------------------------------------

export interface ImageSize {
  width: number;
  height: number;
}

const u16be = (b: Uint8Array, i: number): number => ((b[i] ?? 0) << 8) | (b[i + 1] ?? 0);
const u16le = (b: Uint8Array, i: number): number => (b[i] ?? 0) | ((b[i + 1] ?? 0) << 8);
const u24le = (b: Uint8Array, i: number): number => (b[i] ?? 0) | ((b[i + 1] ?? 0) << 8) | ((b[i + 2] ?? 0) << 16);
const u32be = (b: Uint8Array, i: number): number => (((b[i] ?? 0) << 24) | ((b[i + 1] ?? 0) << 16) | ((b[i + 2] ?? 0) << 8) | (b[i + 3] ?? 0)) >>> 0;
const i32le = (b: Uint8Array, i: number): number => (b[i] ?? 0) | ((b[i + 1] ?? 0) << 8) | ((b[i + 2] ?? 0) << 16) | ((b[i + 3] ?? 0) << 24);
const u32le = (b: Uint8Array, i: number): number => i32le(b, i) >>> 0;
const ascii = (b: Uint8Array, i: number, s: string): boolean => [...s].every((c, k) => b[i + k] === c.charCodeAt(0));

/**
 * The pixel size a picture declares, read from its header bytes alone: PNG, JPEG, GIF, WebP and BMP. null when the
 * format is not one of those or the header is cut short (the caller refuses what it cannot measure). Never throws.
 */
export function imageSize(b: Uint8Array): ImageSize | null {
  // PNG: signature, then IHDR first: width and height as 32-bit big-endian.
  if (b.length >= 24 && b[0] === 0x89 && ascii(b, 1, "PNG") && ascii(b, 12, "IHDR")) return { width: u32be(b, 16), height: u32be(b, 20) };
  // GIF: logical screen width and height, 16-bit little-endian.
  if (b.length >= 10 && (ascii(b, 0, "GIF87a") || ascii(b, 0, "GIF89a"))) return { width: u16le(b, 6), height: u16le(b, 8) };
  // BMP: BITMAPCOREHEADER (12) has 16-bit sizes; the others have signed 32-bit, a negative height meaning top-down.
  if (b.length >= 26 && ascii(b, 0, "BM")) {
    return u32le(b, 14) === 12 ? { width: u16le(b, 18), height: u16le(b, 20) } : { width: Math.abs(i32le(b, 18)), height: Math.abs(i32le(b, 22)) };
  }
  // WebP: RIFF container; lossy (VP8 ), lossless (VP8L) or extended (VP8X) first chunk.
  if (b.length >= 30 && ascii(b, 0, "RIFF") && ascii(b, 8, "WEBP")) {
    if (ascii(b, 12, "VP8X")) return { width: u24le(b, 24) + 1, height: u24le(b, 27) + 1 };
    if (ascii(b, 12, "VP8L") && b[20] === 0x2f) {
      const bits = (b[21] ?? 0) | ((b[22] ?? 0) << 8) | ((b[23] ?? 0) << 16) | ((b[24] ?? 0) << 24);
      return { width: (bits & 0x3fff) + 1, height: ((bits >>> 14) & 0x3fff) + 1 };
    }
    if (ascii(b, 12, "VP8 ") && b[23] === 0x9d && b[24] === 0x01 && b[25] === 0x2a) return { width: u16le(b, 26) & 0x3fff, height: u16le(b, 28) & 0x3fff };
    return null;
  }
  // JPEG: walk the marker segments to the first start-of-frame; its height and width are 16-bit big-endian.
  if (b.length >= 4 && b[0] === 0xff && b[1] === 0xd8) {
    let i = 2;
    while (i + 4 <= b.length) {
      if (b[i] !== 0xff) return null;
      const marker = b[i + 1] ?? 0;
      if (marker === 0xff) {
        i += 1; // fill byte
        continue;
      }
      if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd8)) {
        i += 2; // markers without a length
        continue;
      }
      if (marker === 0xd9 || marker === 0xda) return null; // end of image, or the scan began with no frame header
      const isFrame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
      if (isFrame) return i + 9 <= b.length ? { width: u16be(b, i + 7), height: u16be(b, i + 5) } : null;
      i += 2 + u16be(b, i + 2);
    }
  }
  return null;
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

/** Why a picture of this declared size (null: unreadable header) is not decoded, or null when it may be. */
export function pictureSizeProblem(size: ImageSize | null): string | null {
  if (!size) return "That kind of picture cannot be checked here. Use a PNG or JPEG.";
  if (!(size.width > 0) || !(size.height > 0)) return "That picture could not be opened.";
  if (size.width * size.height > MAX_SCAN_PIXELS) return "That picture has too many pixels to open safely. Use a smaller one, such as a screenshot of the code.";
  return null;
}

/** Decode one frame of pixels. Never throws: a broken decoder reads as "no code found". */
export function decodeFrame(decode: QrDecoder, rgba: Uint8ClampedArray, width: number, height: number): string | null {
  if (!(width > 0) || !(height > 0) || rgba.length !== width * height * 4) return null;
  try {
    return decode(rgba, width, height);
  } catch {
    return null;
  }
}

// --- The camera, opened the right way round ---------------------------------------------------------------------------

export interface CaptureDeps {
  /** Loads the QR decoder (a lazy chunk that can fail to load). */
  decoder(): Promise<QrDecoder>;
  /** `getUserMedia`: asks the person, then switches the camera on. */
  media(): Promise<MediaStream>;
  /** The view was closed meanwhile. */
  stopped(): boolean;
}

/**
 * Get the decoder, then the camera, in that order. A decoder that cannot be loaded (a tab left open across a node
 * update, so the chunk's name is gone) fails here before the camera is ever asked for, so there is no pending permission
 * prompt that could switch a camera on after the page already gave up. A camera that arrives after the view was closed
 * is switched off at once and reads as null.
 */
export async function openCapture(deps: CaptureDeps): Promise<{ decode: QrDecoder; stream: MediaStream } | null> {
  const decode = await deps.decoder();
  if (deps.stopped()) return null;
  const stream = await deps.media();
  if (deps.stopped()) {
    stream.getTracks().forEach((t) => t.stop());
    return null;
  }
  return { decode, stream };
}

/** Read a QR code out of a picture file. */
export async function scanFile(file: File, decode?: QrDecoder): Promise<ScanResult> {
  if (file.size > MAX_SCAN_FILE_BYTES) return { ok: false, problem: "That picture is too large. Use a smaller one." };
  if (file.type !== "" && !file.type.startsWith("image/")) return { ok: false, problem: "That file is not a picture." };
  // The size comes from the header first: createImageBitmap decodes at full size, and bytes say nothing about pixels.
  let size: ImageSize | null = null;
  try {
    size = imageSize(new Uint8Array(await file.slice(0, IMAGE_HEAD_BYTES).arrayBuffer()));
  } catch {
    return { ok: false, problem: "That picture could not be opened." };
  }
  const refused = pictureSizeProblem(size);
  if (refused) return { ok: false, problem: refused };
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
