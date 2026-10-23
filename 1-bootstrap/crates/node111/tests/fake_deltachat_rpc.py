"""A scripted fake of `deltachat-rpc-server` (the chatmail core, pinned v2.63.0) for
node111's `tests/bridge_deltachat.rs` (Layer 6 C5). Stdlib only; run with `python3 -I`.

It speaks what the netns spike recorded (plan/layers/6-transport/research/08-spike-
deltachat.md §7): JSON-RPC 2.0, one object per line both ways, positional params, logs
never on stdout. `get_next_event_batch` long-polls: polls queue in issue order and are
answered only when events exist, at most 101 events each (the core's cap), so their
answers come **after** later requests (out of order, matched by id). The event objects
are `{"contextId": <account>, "event": {"kind": …}}` with the kinds the adapter uses
(IncomingMsg, MsgDelivered, MsgRead, MsgFailed, SecurejoinInviterProgress,
ConfigureProgress, ConnectivityChanged, Info, Warning, EventChannelOverflow) plus
harmless ones (MsgsChanged, ContactsChanged, ChatModified).

The test drives the scripted world with **control lines** written into the same stdin
(objects without a `jsonrpc` key; the adapter never writes one): `join`, `group`,
`incoming`, `deliver`/`read`/`fail` (`msg` or the last send), `hidden_delivered`,
`auto`, `garbage`, `info`, `overflow`, `delay`, `exit`. Every request and its answer
are appended to `--log` as JSON lines. The world persists in
`$DC_ACCOUNTS_PATH/fake-state.json`, so a respawn finds the configured account.
"""

import heapq
import json
import os
import select
import sys
import time
import urllib.parse

OUT = sys.stdout.buffer
BATCH_MAX = 101


def emit(line):
    if not isinstance(line, (bytes, bytearray)):
        line = json.dumps(line, separators=(",", ":")).encode()
    OUT.write(bytes(line) + b"\n")
    OUT.flush()


class Fail(Exception):
    pass


class World:
    def __init__(self, path, log_path):
        self.path = path
        self.log_path = log_path
        self.polls = []  # outstanding get_next_event_batch ids, issue order
        self.events = []
        self.timers = []  # (due, seq, kind, payload)
        self.seq = 0
        self.delays = {}
        self.auto_deliver = False
        self.last_sent = None
        self.state = {"accounts": {}, "next": 10}
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                self.state = json.load(f)

    # -- persistence and log
    def save(self):
        tmp = self.path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(self.state, f)
        os.replace(tmp, self.path)

    def log(self, rec):
        if self.log_path:
            with open(self.log_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(rec) + "\n")

    def next_id(self):
        n = self.state["next"]
        self.state["next"] = n + 1
        return n

    def acct(self, a):
        acc = self.state["accounts"].get(str(a))
        if acc is None:
            raise Fail("no such account")
        return acc

    def only(self):
        ids = sorted(int(k) for k in self.state["accounts"])
        if not ids:
            raise Fail("no account")
        return ids[0], self.state["accounts"][str(ids[0])]

    # -- events
    def event(self, ctx, kind, **fields):
        ev = {"kind": kind}
        ev.update(fields)
        self.events.append({"contextId": ctx, "event": ev})

    def pump(self):
        while self.polls and self.events:
            rid = self.polls.pop(0)
            batch, self.events = self.events[:BATCH_MAX], self.events[BATCH_MAX:]
            self.log({"answer": "get_next_event_batch", "id": rid, "events": len(batch)})
            emit({"jsonrpc": "2.0", "id": rid, "result": batch})

    def later(self, secs, kind, payload):
        self.seq += 1
        heapq.heappush(self.timers, (time.monotonic() + secs, self.seq, kind, payload))

    def due(self):
        now = time.monotonic()
        while self.timers and self.timers[0][0] <= now:
            _, _, kind, payload = heapq.heappop(self.timers)
            if kind == "respond":
                emit(payload)
            elif kind == "state":
                self.set_state(*payload)
        self.pump()

    def timeout(self):
        if not self.timers:
            return None
        return max(0.0, self.timers[0][0] - time.monotonic())

    # -- the scripted world
    def contact_by_addr(self, acc, addr):
        for cid, c in acc["contacts"].items():
            if c["address"] == addr:
                return int(cid)
        return None

    def new_contact(self, acc, addr, name):
        cid = self.next_id()
        acc["contacts"][str(cid)] = {"address": addr, "name": name}
        return cid

    def single_chat(self, acc, cid, create=True):
        for chat_id, ch in acc["chats"].items():
            if ch["type"] == "Single" and ch.get("contact") == cid:
                return int(chat_id)
        if not create:
            return None
        chat_id = self.next_id()
        acc["chats"][str(chat_id)] = {
            "type": "Single",
            "name": acc["contacts"][str(cid)]["name"],
            "contact": cid,
        }
        return chat_id

    def group_chat(self, acc, name):
        for chat_id, ch in acc["chats"].items():
            if ch["type"] == "Group" and ch["name"] == name:
                return int(chat_id)
        chat_id = self.next_id()
        acc["chats"][str(chat_id)] = {"type": "Group", "name": name}
        return chat_id

    def set_state(self, a, mid, state, kind):
        acc = self.acct(a)
        m = acc["msgs"].get(str(mid))
        if m is None:
            return
        m["state"] = state
        self.save()
        self.event(a, kind, chatId=m["chatId"], msgId=mid)
        self.pump()

    # -- control lines
    def control(self, c):
        what = c.get("control")
        a, acc = self.only()
        if what == "join":
            addr, name = c["addr"], c.get("name", "")
            cid = self.contact_by_addr(acc, addr) or self.new_contact(acc, addr, name)
            chat = self.single_chat(acc, cid)
            self.save()
            self.event(a, "ContactsChanged", contactId=cid)
            self.event(
                a,
                "SecurejoinInviterProgress",
                contactId=cid,
                chatType="Single",
                chatId=chat,
                progress=1000,
            )
        elif what == "group":
            chat = self.group_chat(acc, c["name"])
            self.save()
            self.event(a, "ChatModified", chatId=chat)
        elif what == "incoming":
            addr = c["from"]
            cid = self.contact_by_addr(acc, addr) or self.new_contact(
                acc, addr, c.get("name", "")
            )
            if c.get("group"):
                chat = self.group_chat(acc, c["group"])
            else:
                chat = self.single_chat(acc, cid)
            mid = self.next_id()
            acc["msgs"][str(mid)] = {
                "chatId": chat,
                "fromId": cid,
                "text": c.get("text", ""),
                "state": 10,
                "isInfo": bool(c.get("info", False)),
                "fileBytes": int(c.get("file_bytes", 0)),
                "timestamp": 1760000000,
            }
            self.save()
            self.event(a, "MsgsChanged", chatId=chat, msgId=mid)
            self.event(a, "IncomingMsg", chatId=chat, msgId=mid)
        elif what in ("deliver", "read", "fail"):
            mid = c.get("msg", self.last_sent)
            state, kind = {
                "deliver": (26, "MsgDelivered"),
                "read": (28, "MsgRead"),
                "fail": (24, "MsgFailed"),
            }[what]
            self.set_state(a, mid, state, kind)
        elif what == "hidden_delivered":
            # the hidden Secure-Join handshake messages also fire MsgDelivered (spike §6)
            self.event(a, "MsgDelivered", chatId=0, msgId=c.get("msg", 9999))
        elif what == "auto":
            self.auto_deliver = bool(c.get("deliver", True))
        elif what == "info":
            for i in range(int(c.get("n", 1))):
                self.event(a, "Info", msg=f"IMAP-fake {i}")
        elif what == "overflow":
            self.event(a, "EventChannelOverflow", n=int(c.get("n", 1)))
        elif what == "delay":
            self.delays[c["method"]] = float(c.get("secs", 0.2))
        elif what == "garbage":
            emit(b"this is not json")
            emit(b"[1,2,3]")
            emit({"jsonrpc": "2.0", "id": 999999, "result": None})
            emit({"jsonrpc": "2.0", "method": "event", "params": []})
            emit({"jsonrpc": "2.0", "id": 5, "result": 1, "extra": True})
            emit(b"x" * 70000)  # over the 64 KiB line cap: the supervisor drops it
            emit(b"\xff\xfe not utf-8")
            self.events.append({"contextId": a, "event": {"kind": "Telepathy"}})
            self.events.append({"contextId": a})
            self.events.append({"contextId": a, "event": {"kind": "IncomingMsg"}})
            self.event(a, "Warning", msg="The server illegally decreased the uid_next")
        elif what == "exit":
            OUT.flush()
            os._exit(0)
        self.pump()

    # -- methods
    def call(self, method, params):
        m = getattr(self, "m_" + method, None)
        if m is None:
            return None, {"code": -32601, "message": "Method not found"}
        try:
            return m(*params), None
        except Fail as e:
            return None, {"code": -1, "message": str(e)}
        except TypeError as e:
            return None, {"code": -32602, "message": f"Invalid params: {e}"}

    def m_get_system_info(self):
        return {"deltachat_core_version": "v2.63.0", "arch": "64", "num_cpus": "4"}

    def m_get_all_account_ids(self):
        return sorted(int(k) for k in self.state["accounts"])

    def m_add_account(self):
        ids = [int(k) for k in self.state["accounts"]]
        a = max(ids, default=0) + 1
        self.state["accounts"][str(a)] = {
            "config": {},
            "configured": False,
            "addr": None,
            "io": False,
            "contacts": {},
            "chats": {},
            "msgs": {},
        }
        self.save()
        return a

    def m_set_config(self, a, key, value):
        self.acct(a)["config"][key] = value
        self.save()
        return None

    def m_get_config(self, a, key):
        acc = self.acct(a)
        if key == "configured_addr":
            return acc["addr"] if acc["configured"] else None
        return acc["config"].get(key)

    def m_is_configured(self, a):
        return bool(self.acct(a)["configured"])

    def m_add_transport_from_qr(self, a, qr):
        acc = self.acct(a)
        if not qr.startswith("dclogin:"):
            raise Fail("not a dclogin: QR")
        for p in (0, 500, 1000):
            self.event(a, "ConfigureProgress", progress=p, comment=None)
        acc["addr"] = qr[len("dclogin:"):].split("/?", 1)[0]
        acc["configured"] = True
        self.save()
        return None

    def m_start_io(self, a):
        self.acct(a)["io"] = True
        self.event(a, "ConnectivityChanged")
        return None

    def m_wait_for_all_work_done(self, a):
        self.acct(a)
        return None

    def m_get_chat_securejoin_qr_code(self, a, chat):
        acc = self.acct(a)
        q = urllib.parse.quote
        name = acc["config"].get("displayname", "")
        return (
            "https://i.delta.chat/#EDDC26488B55F1335DE32B36854DEAA61FE882F8&v=3&i=inv&s=sec"
            f"&a={q(acc['addr'] or '', safe='')}&n={q(name, safe='')}"
        )

    def m_get_contact(self, a, cid):
        c = self.acct(a)["contacts"].get(str(cid))
        if c is None:
            raise Fail("contact not found")
        return {
            "address": c["address"],
            "color": "#000000",
            "authName": c["name"],
            "status": "",
            "displayName": c["name"] or c["address"],
            "id": cid,
            "name": "",
            "profileImage": None,
            "isBlocked": False,
            "isKeyContact": True,
            "e2eeAvail": True,
            "lastSeen": 0,
            "freshness": "Old",
            "isBot": False,
        }

    def m_get_message(self, a, mid):
        m = self.acct(a)["msgs"].get(str(mid))
        if m is None:
            raise Fail("message not found")
        return {
            "id": mid,
            "chatId": m["chatId"],
            "fromId": m["fromId"],
            "text": m["text"],
            "state": m["state"],
            "isInfo": m["isInfo"],
            "isBot": m["fromId"] == 1,
            "showPadlock": True,
            "viewType": "Text",
            "fileBytes": m["fileBytes"],
            "timestamp": m["timestamp"],
        }

    def m_get_basic_chat_info(self, a, chat):
        ch = self.acct(a)["chats"].get(str(chat))
        if ch is None:
            raise Fail("chat not found")
        return {"id": chat, "name": ch["name"], "chatType": ch["type"], "isContactRequest": False}

    def m_get_chat_id_by_contact_id(self, a, cid):
        return self.single_chat(self.acct(a), cid, create=False)

    def m_create_chat_by_contact_id(self, a, cid):
        acc = self.acct(a)
        if str(cid) not in acc["contacts"]:
            raise Fail("contact not found")
        chat = self.single_chat(acc, cid)
        self.save()
        return chat

    def m_misc_send_text_message(self, a, chat, text):
        acc = self.acct(a)
        if str(chat) not in acc["chats"]:
            raise Fail("chat not found")
        mid = self.next_id()
        acc["msgs"][str(mid)] = {
            "chatId": chat,
            "fromId": 1,
            "text": text,
            "state": 20,
            "isInfo": False,
            "fileBytes": 0,
            "timestamp": 1760000000,
        }
        self.save()
        self.last_sent = mid
        self.event(a, "MsgsChanged", chatId=chat, msgId=mid)
        if self.auto_deliver:
            self.later(0.05, "state", (a, mid, 26, "MsgDelivered"))
        return mid

    def m_markseen_msgs(self, a, ids):
        self.acct(a)
        return None

    # -- one input line
    def line(self, raw):
        try:
            obj = json.loads(raw)
        except ValueError:
            return
        if not isinstance(obj, dict):
            return
        if "jsonrpc" not in obj and "control" in obj:
            self.control(obj)
            return
        rid, method, params = obj.get("id"), obj.get("method"), obj.get("params", [])
        self.log({"method": method, "params": params, "id": rid})
        if method == "get_next_event_batch":
            self.polls.append(rid)
            self.pump()
            return
        result, error = self.call(method, params if isinstance(params, list) else [])
        reply = {"jsonrpc": "2.0", "id": rid}
        if error is None:
            reply["result"] = result
        else:
            reply["error"] = error
        self.log({"answer": method, "id": rid, "result": result, "error": error})
        delay = self.delays.get(method)
        if delay:
            self.later(delay, "respond", reply)
        else:
            emit(reply)
        self.pump()


def main():
    log_path = None
    args = sys.argv[1:]
    if len(args) >= 2 and args[0] == "--log":
        log_path = args[1]
    accounts = os.environ.get("DC_ACCOUNTS_PATH", ".")
    os.makedirs(accounts, exist_ok=True)
    world = World(os.path.join(accounts, "fake-state.json"), log_path)
    buf = b""
    while True:
        r, _, _ = select.select([0], [], [], world.timeout())
        if r:
            chunk = os.read(0, 65536)
            if not chunk:
                return  # stdin closed: exit cleanly, like the real server
            buf += chunk
            while b"\n" in buf:
                raw, buf = buf.split(b"\n", 1)
                if raw.strip():
                    world.line(raw.decode("utf-8", "replace"))
        world.due()


if __name__ == "__main__":
    main()
