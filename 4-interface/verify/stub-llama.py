#!/usr/bin/env python3
"""Stub llama-server for 4-interface/verify/smoke.sh (OpenAI-compatible, stdlib only).

Origin: the Layer 4 scouts' mock (scratchpad mockllama.py), owned here since I1.
Accepts llama-server's own flags so 111d can supervise it as current/bin/llama-server:
    stub-llama.py --model X.gguf --host 127.0.0.1 --port 11181
    stub-llama.py 11181                      (legacy positional port)

Routes:
    GET  /health                 200 {"status":"ok"}  (what 111-agent and 111d probe)
    GET  /v1/models              one fake model
    POST /v1/chat/completions    scripted replies, streamed as OpenAI SSE deltas
    stub-llama.py … --tool-log FILE          (smoke.sh --three-node: one JSON line per tool
                                             result the agent sends back, {name, content})
Script (keyed on the LAST user message, lowercased):
    "boom"                        -> 500 {"error":"upstream exploded"} (pre-stream failure)
    last message role == tool     -> "Per the KB: found it." (and the result goes to --tool-log)
    "@tool NAME {json}" + tools   -> one streamed NAME tool call with those arguments (Layer 5:
                                     the smoke drives messages.read / messages.send / contacts.read /
                                     fabric.read / nearby.read through the real 111-agent)
    contains "meshtastic" + tools -> text + one streamed kb.read tool call (args split over 2 deltas)
    request has no tools          -> "## Summary\\nstub diagnosis\\n" (agent task path)
    anything else                 -> "Hello there" in 2 deltas
`"stream": false` gets the same text as one chat.completion JSON object.
"""
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


TOOL_LOG = None
TOOL_DIRECTIVE = re.compile(r"@tool\s+([A-Za-z_.]+)\s*(\{.*\})?\s*$", re.S)


def parse_args(argv):
    global TOOL_LOG
    host, port = "127.0.0.1", 11181
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in ("--host", "--port", "--model", "-m", "--tool-log") and i + 1 < len(argv):
            if a == "--host":
                host = argv[i + 1]
            elif a == "--port":
                port = int(argv[i + 1])
            elif a == "--tool-log":
                TOOL_LOG = argv[i + 1]
            i += 2
            continue
        if a.isdigit():
            port = int(a)
        i += 1
    return host, port


def log_tool_result(msgs):
    """Append the trailing tool results (with their call's name) to --tool-log."""
    if not TOOL_LOG:
        return
    names = {}
    for m in msgs:
        for c in m.get("tool_calls") or []:
            names[c.get("id")] = (c.get("function") or {}).get("name")
    tail = []
    for m in reversed(msgs):
        if m.get("role") != "tool":
            break
        tail.append(m)
    with open(TOOL_LOG, "a", encoding="utf-8") as f:
        for m in reversed(tail):
            f.write(json.dumps({"name": names.get(m.get("tool_call_id")), "content": m.get("content")}) + "\n")


def text_chunks(text):
    """Split into 2-3 deltas so the relay has several token frames to translate."""
    words = text.split(" ")
    if len(words) < 2:
        return [text]
    mid = len(words) // 2
    return [" ".join(words[:mid]) + " ", " ".join(words[mid:])]


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send_json(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path == "/health":
            self.send_json(200, {"status": "ok"})
        elif self.path == "/v1/models":
            self.send_json(200, {"object": "list", "data": [{"id": "stub", "object": "model"}]})
        else:
            self.send_response(404)
            self.send_header("content-length", "0")
            self.end_headers()

    def sse(self, chunks):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("connection", "close")
        self.end_headers()
        for c in chunks:
            self.wfile.write(b"data: " + json.dumps(c).encode() + b"\n\n")
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True

    def do_POST(self):
        n = int(self.headers.get("content-length", "0"))
        try:
            body = json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            self.send_json(400, {"error": "bad json"})
            return
        msgs = body.get("messages") or []
        tools = body.get("tools")
        stream = body.get("stream", False)
        sys.stderr.write(
            "stub-llama: roles=%s tools=%s stream=%s\n"
            % ([m.get("role") for m in msgs], [t["function"]["name"] for t in tools] if tools else None, stream)
        )
        sys.stderr.flush()
        users = [m for m in msgs if m.get("role") == "user"]
        user = (users[-1].get("content") or "").lower() if users else ""
        last = msgs[-1] if msgs else {"role": "user"}

        if "boom" in user:
            self.send_json(500, {"error": "upstream exploded"})
            return
        tool_call = None
        directive = TOOL_DIRECTIVE.search(users[-1].get("content") or "") if users else None
        if last.get("role") == "tool":
            log_tool_result(msgs)
            text = "Per the KB: found it."
        elif directive and tools:
            args = directive.group(2) or "{}"
            cut = max(1, len(args) // 2)
            text = "Let me look. "
            tool_call = ("c1", directive.group(1), args[:cut], args[cut:])
        elif "meshtastic" in user and tools:
            text = "Let me check. "
            tool_call = ("c1", "kb.read", '{"query":', '"meshtastic"}')
        elif not tools:
            text = "## Summary\nstub diagnosis\n"
        else:
            text = "Hello there"

        if not stream:
            msg = {"role": "assistant", "content": text}
            if tool_call:
                msg["tool_calls"] = [{"id": tool_call[0], "type": "function",
                                      "function": {"name": tool_call[1], "arguments": tool_call[2] + tool_call[3]}}]
            self.send_json(200, {"object": "chat.completion", "model": "stub",
                                 "choices": [{"index": 0, "message": msg,
                                              "finish_reason": "tool_calls" if tool_call else "stop"}]})
            return
        chunks = [{"choices": [{"delta": {"content": c}}]} for c in text_chunks(text)]
        if tool_call:
            cid, name, a1, a2 = tool_call
            chunks.append({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "id": cid, "function": {"name": name, "arguments": a1}}]}}]})
            chunks.append({"choices": [{"delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": a2}}]}}]})
        self.sse(chunks)


if __name__ == "__main__":
    host, port = parse_args(sys.argv[1:])
    sys.stderr.write("stub-llama: listening on %s:%d\n" % (host, port))
    sys.stderr.flush()
    ThreadingHTTPServer((host, port), H).serve_forever()
