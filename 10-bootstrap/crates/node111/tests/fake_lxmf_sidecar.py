"""A scripted stand-in for the LXMF helper (Layer 6 spec section 6; the protocol of
60-transport/sidecars/lxmf-sidecar/README.md): JSON Lines over piped stdio, for the
node111 adapter tests (tests/bridge_lxmf.rs). No Reticulum, no network, no sockets.

    python3 -I fake_lxmf_sidecar.py <scenario.json> <record-file> [ignored helper args...]

The scenario is {"runs": [[step, ...], ...]}: spawn i runs runs[min(i, len - 1)] (the spawn
count lives in <record-file>.runs, so a supervisor respawn plays the next run). Every stdin
line is appended to <record-file> exactly as received (bytes and newline), so the tests can
byte-assert what 111d wrote. Steps:

    {"emit": {...}}                  write the object as one compact JSON line
    {"raw": "text"}                  write the text and a newline as given (malformed lines)
    {"long": N}                      write a line of N bytes ("x" repeated)
    {"expect": "send", "as": "s1"}   read stdin until a line of that type; keep it as s1
    {"state": {"ref": "s1", ...}}    write a state line for s1's id (keys: hash, state, reason)
    {"heard_of": "s1", "name": ..}   write a heard line for s1's destination
    {"sleep_ms": N}
    {"exit": code}                   exit now with that code
After the last step the fake reads stdin until "quit" or end of input and exits 0.
"""
import json
import os
import sys
import time


def main():
    scenario_path, record = sys.argv[1], sys.argv[2]
    with open(scenario_path, "r", encoding="utf-8") as f:
        runs = json.load(f)["runs"]
    counter = record + ".runs"
    try:
        with open(counter, "r") as f:
            n = int(f.read().strip() or "0")
    except OSError:
        n = 0
    with open(counter, "w") as f:
        f.write(str(n + 1))
    steps = runs[min(n, len(runs) - 1)]
    out = sys.stdout.buffer
    inp = sys.stdin.buffer
    saved = {}

    def write(raw):
        out.write(raw + b"\n")
        out.flush()

    def emit(obj):
        write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))

    def read_line():
        raw = inp.readline()
        if not raw:
            return None
        with open(record, "ab") as f:
            f.write(raw)
        try:
            return json.loads(raw.decode("utf-8"))
        except ValueError:
            return {}

    def expect(kind):
        while True:
            obj = read_line()
            if obj is None:
                sys.exit(0)
            if obj.get("type") == "quit" and kind != "quit":
                sys.exit(0)
            if kind == "any" or obj.get("type") == kind:
                return obj

    for step in steps:
        if "emit" in step:
            emit(step["emit"])
        elif "raw" in step:
            write(step["raw"].encode("utf-8"))
        elif "long" in step:
            write(b"x" * int(step["long"]))
        elif "expect" in step:
            obj = expect(step["expect"])
            if "as" in step:
                saved[step["as"]] = obj
            if step["expect"] == "quit":
                sys.exit(0)
        elif "state" in step:
            spec = dict(step["state"])
            line = {"type": "state", "id": saved[spec.pop("ref")]["id"]}
            for key in ("hash", "state", "reason"):
                if key in spec:
                    line[key] = spec[key]
            emit(line)
        elif "heard_of" in step:
            emit({"type": "heard", "dest": saved[step["heard_of"]]["to"], "name": step.get("name")})
        elif "sleep_ms" in step:
            time.sleep(int(step["sleep_ms"]) / 1000.0)
        elif "exit" in step:
            out.flush()
            os._exit(int(step["exit"]))
    while True:
        obj = read_line()
        if obj is None or obj.get("type") == "quit":
            sys.exit(0)


if __name__ == "__main__":
    main()
