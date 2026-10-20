// Placeholder Meshtastic web client transport (4-interface/verify/cdp): the HTTP API of the
// real client, against its own origin only. The downlink frames are JSON here (the lint's
// stub meshmud stands protobufs in with JSON; 111d never decodes either).
"use strict";
const state = { polls: 0, frames: 0, errors: 0, nodes: [], board: null, complete: false, putOk: false, lastStatus: 0 };
const enc = new TextEncoder();
const dec = new TextDecoder();

function publish(phase) {
  document.body.dataset.state = phase;
  document.body.dataset.nodes = String(state.nodes.length);
  document.body.dataset.board = state.board || "";
  document.body.dataset.polls = String(state.polls);
  document.body.dataset.frames = String(state.frames);
  document.body.dataset.errors = String(state.errors);
  document.body.dataset.put = state.putOk ? "ok" : "no";
  document.getElementById("status").textContent =
    phase + ": " + state.frames + " frames, " + state.nodes.length + " nodes";
  const ul = document.getElementById("nodes");
  ul.replaceChildren(
    ...state.nodes.map((n) => {
      const li = document.createElement("li");
      li.textContent = n.long_name + " (" + n.id + ")"; // text only: names are RF-origin strings
      return li;
    }),
  );
}

async function putToRadio(bytes) {
  const r = await fetch("/api/v1/toradio", { method: "PUT", body: bytes, headers: { "Content-Type": "application/x-protobuf" } });
  state.lastStatus = r.status;
  state.putOk = r.ok;
  if (!r.ok) state.errors++;
}

// one frame per call whatever `all` says: poll until an empty answer
async function drain() {
  for (let i = 0; i < 64; i++) {
    const r = await fetch("/api/v1/fromradio?all=true");
    state.polls++;
    state.lastStatus = r.status;
    if (!r.ok) {
      state.errors++;
      return;
    }
    const body = new Uint8Array(await r.arrayBuffer());
    if (body.length === 0) return;
    state.frames++;
    try {
      const msg = JSON.parse(dec.decode(body));
      if (msg.my_node_info) state.board = msg.my_node_info.long_name;
      if (msg.node_info) state.nodes = state.nodes.filter((n) => n.num !== msg.node_info.num).concat([msg.node_info]);
      if (msg.config_complete_id) state.complete = true;
    } catch {
      state.errors++;
    }
  }
}

async function main() {
  document.getElementById("host").value = location.host; // the connect dialog prefill
  publish("connecting");
  try {
    await putToRadio(enc.encode("want_config"));
  } catch {
    state.errors++;
  }
  for (;;) {
    try {
      await drain();
    } catch {
      state.errors++;
    }
    publish(state.complete ? "connected" : "connecting");
    await new Promise((r) => setTimeout(r, 1000));
  }
}

main();
