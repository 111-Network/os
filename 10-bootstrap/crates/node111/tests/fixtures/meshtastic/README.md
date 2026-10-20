# Meshtastic `FromRadio` parity fixture (Layer 6 §5 "Nearby folds into the client", §16 G2)

- `parity.fromradio` — one board's stream as meshmud forwards it: `0x94 0xC3 <len>` frames of
  prost-encoded `FromRadio` built from the `meshtastic` crate's types (`parity_frames()` in
  `tests/meshtastic_client.rs`; `the_parity_fixture_is_current` keeps the two equal, regenerate
  with `MESHTASTIC_FIXTURE_WRITE=1`). Config stream: `my_info` (`!0a0a0a0a`), the board's own
  NodeInfo, four NodeInfos (a full one; a hostile 40-byte name with a bidi override and a control
  sequence in the short name; a bare one with no `User`; a multibyte name over 24 bytes with an
  out-of-range altitude and hop count), two channels (the public default key; `Ops` keyed with the
  sentinel PSK of the leak test), LoRa config, a security config, `config_complete_id`. Live
  packets: a text, a position, two telemetry packets, a NodeInfo packet from a new node, a PKI DM
  from an unknown node, a routing ack.
- `parity.expected.json` — what the presence reader writes for that stream: meshtastic-python
  2.7.11 (the reader's hash-locked closure, `meshtastic-presence-lock.txt`) fed the frames through
  `MeshInterface._handleFromRadio`, then the reader's own `normalize_nodes` and `encode_document`
  (`meshtastic_presence.py`). Produced once on 2026-10-19 in a scratch venv; nothing here imports
  Python at test time. Names are compared cut to 24 bytes (the reader cuts at 40, the protobuf
  limit is 24).
