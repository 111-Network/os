// Streaming Server-Sent Events parser for fetch bodies (EventSource cannot POST).
// Implements the WHATWG "interpreting an event stream" rules: LF / CRLF / lone CR line ends,
// multi-line `data:`, comment lines, one leading space stripped from values, blank-line dispatch,
// UTF-8 decoded incrementally so multi-byte characters may straddle chunk boundaries.

export interface SseMessage {
  /** `event:` field; undefined means the default type ("message"). */
  event?: string;
  /**
   * The stream's last event id (spec: it persists across events until a new `id:` arrives).
   * Undefined until the first `id:` line.
   */
  id?: string;
  /** `data:` lines joined with "\n" (may be an empty string). */
  data: string;
  /** `retry:` field, when it was all digits. */
  retry?: number;
}

type ByteSource = ReadableStreamDefaultReader<Uint8Array> | AsyncIterable<Uint8Array>;

function isReader(s: ByteSource): s is ReadableStreamDefaultReader<Uint8Array> {
  return typeof (s as ReadableStreamDefaultReader<Uint8Array>).read === "function";
}

async function* chunksOf(source: ByteSource): AsyncGenerator<Uint8Array> {
  if (isReader(source)) {
    const reader = source;
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        if (value) yield value;
      }
    } finally {
      // Consumer stopped early (or the stream ended): release the connection.
      try {
        await reader.cancel();
      } catch {
        /* already closed */
      }
    }
  } else {
    yield* source;
  }
}

/** Parse an SSE byte stream into messages. Incomplete trailing events are discarded (per spec). */
export async function* parseSseStream(source: ByteSource): AsyncGenerator<SseMessage, void, void> {
  const decoder = new TextDecoder("utf-8");
  let buffer = "";
  let data: string[] = [];
  let event: string | undefined;
  let lastId: string | undefined;
  let retry: number | undefined;

  function* handleLine(line: string): Generator<SseMessage, void, void> {
    if (line === "") {
      if (data.length > 0) {
        const msg: SseMessage = { data: data.join("\n") };
        if (event !== undefined && event !== "") msg.event = event;
        if (lastId !== undefined) msg.id = lastId;
        if (retry !== undefined) msg.retry = retry;
        yield msg;
      }
      data = [];
      event = undefined;
      retry = undefined;
      return;
    }
    if (line.startsWith(":")) return;
    const colon = line.indexOf(":");
    const field = colon < 0 ? line : line.slice(0, colon);
    let value = colon < 0 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) value = value.slice(1);
    switch (field) {
      case "event":
        event = value;
        break;
      case "data":
        data.push(value);
        break;
      case "id":
        if (!value.includes("\0")) lastId = value;
        break;
      case "retry":
        if (/^\d+$/.test(value)) retry = Number(value);
        break;
      default:
        break; // unknown fields are ignored
    }
  }

  function* drain(final: boolean): Generator<SseMessage, void, void> {
    let start = 0;
    for (;;) {
      let end = -1;
      for (let k = start; k < buffer.length; k++) {
        const c = buffer.charCodeAt(k);
        if (c === 10 || c === 13) {
          end = k;
          break;
        }
      }
      if (end < 0) break;
      let next = end + 1;
      if (buffer.charCodeAt(end) === 13) {
        if (next >= buffer.length && !final) break; // a lone CR at the chunk end may be half a CRLF
        if (buffer.charCodeAt(next) === 10) next++;
      }
      yield* handleLine(buffer.slice(start, end));
      start = next;
    }
    buffer = buffer.slice(start);
  }

  for await (const chunk of chunksOf(source)) {
    buffer += decoder.decode(chunk, { stream: true });
    yield* drain(false);
  }
  buffer += decoder.decode();
  yield* drain(true);
  // Whatever is left is an unterminated line / event: discarded, as EventSource does.
}
