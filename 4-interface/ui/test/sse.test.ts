import { describe, expect, it, vi } from "vitest";
import { parseSseStream, type SseMessage } from "../src/lib/api/sse";
import { bytes, chunksOf, collect, splitBytes, streamOf } from "./helpers/fakes";

async function parse(text: string, offsets: number[] = []): Promise<SseMessage[]> {
  return collect(parseSseStream(chunksOf(splitBytes(text, offsets))));
}

const STREAM =
  ": hello\n" +
  "event: token\ndata: {\"text\":\"Hi \"}\n\n" +
  "event: tool\ndata: {\"name\":\"kb.read\"}\n\n" +
  "id: 7\nevent: done\ndata: {\"reply\":\"ok\"}\n\n";

describe("parseSseStream", () => {
  it("parses named events with data and id", async () => {
    expect(await parse(STREAM)).toEqual([
      { event: "token", data: '{"text":"Hi "}' },
      { event: "tool", data: '{"name":"kb.read"}' },
      { event: "done", id: "7", data: '{"reply":"ok"}' },
    ]);
  });

  it("gives the same result when split at every byte offset", async () => {
    const expected = await parse(STREAM);
    const total = bytes(STREAM).length;
    for (let k = 1; k < total; k++) expect(await parse(STREAM, [k])).toEqual(expected);
  });

  it("gives the same result with one byte per chunk and with every offset cut", async () => {
    const expected = await parse(STREAM);
    const total = bytes(STREAM).length;
    expect(await parse(STREAM, Array.from({ length: total }, (_, i) => i))).toEqual(expected);
  });

  it("handles CRLF and lone CR line endings", async () => {
    const crlf = "event: token\r\ndata: a\r\n\r\nevent: done\r\ndata: b\r\n\r\n";
    expect(await parse(crlf)).toEqual([
      { event: "token", data: "a" },
      { event: "done", data: "b" },
    ]);
    const cr = "event: token\rdata: a\r\rdata: b\r\r";
    expect(await parse(cr)).toEqual([{ event: "token", data: "a" }, { data: "b" }]);
  });

  it("treats a CRLF split across two chunks as one line ending", async () => {
    // Misreading "\r" + "\n" as two terminators would dispatch after "a" and yield two events.
    const text = "data: a\r\ndata: b\r\n\r\n";
    const cut = text.indexOf("\r") + 1; // between CR and LF
    expect(await parse(text, [cut])).toEqual([{ data: "a\nb" }]);
    for (let k = 1; k < text.length; k++) expect(await parse(text, [k])).toEqual([{ data: "a\nb" }]);
  });

  it("joins multi-line data with newlines", async () => {
    expect(await parse("data: one\ndata: two\ndata: three\n\n")).toEqual([{ data: "one\ntwo\nthree" }]);
  });

  it("dispatches an event whose data is empty, but not one with no data field", async () => {
    expect(await parse("data:\n\n")).toEqual([{ data: "" }]);
    expect(await parse("data\n\n")).toEqual([{ data: "" }]);
    expect(await parse("event: x\n\n")).toEqual([]);
    expect(await parse("id: 3\n\n")).toEqual([]);
  });

  it("ignores comment lines, including keep-alives between events", async () => {
    const text = ": keep-alive\n\n: another\n" + "event: a\ndata: 1\n\n" + ": keep-alive\n\n" + "data: 2\n\n";
    expect(await parse(text)).toEqual([{ event: "a", data: "1" }, { data: "2" }]);
  });

  it("strips exactly one leading space from values", async () => {
    expect(await parse("data:x\n\ndata: x\n\ndata:  x\n\n")).toEqual([{ data: "x" }, { data: "x" }, { data: " x" }]);
  });

  it("keeps colons inside values and ignores unknown fields", async () => {
    expect(await parse('foo: bar\ndata: {"a":"b:c"}\n\n')).toEqual([{ data: '{"a":"b:c"}' }]);
  });

  it("resets the event type per event but keeps the last id (spec)", async () => {
    expect(await parse("id: 1\nevent: a\ndata: x\n\ndata: y\n\nid: 2\ndata: z\n\n")).toEqual([
      { event: "a", id: "1", data: "x" },
      { id: "1", data: "y" },
      { id: "2", data: "z" },
    ]);
  });

  it("ignores ids containing NUL and parses retry only when numeric", async () => {
    expect(await parse("id: a\0b\ndata: x\n\n")).toEqual([{ data: "x" }]);
    expect(await parse("retry: 1500\ndata: x\n\nretry: soon\ndata: y\n\n")).toEqual([{ retry: 1500, data: "x" }, { data: "y" }]);
  });

  it("decodes multi-byte UTF-8 split across chunk boundaries", async () => {
    const text = 'event: token\ndata: {"text":"café 日本語 🙂"}\n\n';
    const expected = [{ event: "token", data: '{"text":"café 日本語 🙂"}' }];
    const total = bytes(text).length;
    for (let k = 1; k < total; k++) expect(await parse(text, [k])).toEqual(expected);
    // all bytes separate, so every multi-byte sequence is torn apart
    expect(await parse(text, Array.from({ length: total }, (_, i) => i))).toEqual(expected);
  });

  it("strips a leading byte order mark", async () => {
    expect(await parse("﻿data: x\n\n")).toEqual([{ data: "x" }]);
  });

  it("discards an event that never got its blank line", async () => {
    expect(await parse("data: complete\n\ndata: cut off")).toEqual([{ data: "complete" }]);
    expect(await parse("data: complete\n\ndata: half\n")).toEqual([{ data: "complete" }]);
  });

  it("reads from a ReadableStream reader and cancels it when the consumer stops early", async () => {
    const cancelled = vi.fn();
    const reader = streamOf(splitBytes(STREAM, [5, 40]), cancelled, true).getReader();
    const seen: string[] = [];
    for await (const m of parseSseStream(reader)) {
      seen.push(m.event ?? "");
      if (seen.length === 2) break;
    }
    expect(seen).toEqual(["token", "tool"]);
    expect(cancelled).toHaveBeenCalled();
  });

  it("completes normally on an empty stream", async () => {
    expect(await collect(parseSseStream(chunksOf([])))).toEqual([]);
    expect(await collect(parseSseStream(streamOf([]).getReader()))).toEqual([]);
  });
});
