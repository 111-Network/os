---
title: Preferred Networks
category: concept
summary: Ordered list of preferred networks, read as per-peer path preference.
tags: [mesh, bridge, networking]
aliases: [network order, preferred network list]
sources:
  - https://github.com/111-Network/os/blob/main/README.md
relations:
  relates_to: ["[[lxmf]]", "[[meshtastic]]", "[[meshcore]]", "[[delta-chat-securejoin]]"]
---

Ordered list of preferred networks, read as per-peer path preference.

## What it is

The root README describes a short, ordered list of preferred networks, curated in the Knowledge Layer: the node joins
them in sequence, starts heartbeats and data exchange on the first one found, and falls back through the list.
Layer 6 stores the list as the `[order]` table of `interfaces.toml`, using network ids from the foreign message model.

## Use when

- Deciding the default order the Messages rail, Settings, and the agent's setup proposals present networks in.
- Explaining which networks a node reaches and in what sequence a user would want them tried.

## Do NOT use when

- Joining in sequence is meant: Layer 7 builds no join-in-sequence state machine; presence is announced on link-up and
  in cards, and the order only ranks paths per peer.
- A network is not enabled: the order lists preferences, and an off network stays off.

## Details

Default order (Layer 6 spec section 11):

1. `111`: the default fabric, always first.
2. `lxmf`: Reticulum, reached through the LXMF helper; Reticulum has no entry of its own in the order, see [[lxmf]].
3. `meshtastic`: the board 111 drives through the multiplexer ([[meshtastic]]).
4. `app:deltachat`, then `app:simplex`: app peers, which also decrypt on the hub ([[delta-chat-securejoin]]).

`[order]` is path preference (Layer 7 §19), not a join-in-sequence state machine: the node keeps every enabled network
on, in parallel, and tries them in the user's preferred order per peer. Today the order sorts the rail, Settings and
agent proposals. The user can reorder; the KB supplies the default and the reasoning. MeshCore is a transport option
([[meshcore]]) and not an entry of the default list.

## Evidence

- Source of the idea: the root README ("Existing mesh networks are treated as pure transport").
- Order and ids from the Layer 6 spec sections 3 and 11; Layer 7 (`7-relay/README.md` §19) reads the order as path
  preference and builds no sequence state machine.

## Links

- Reticulum carrier: [[lxmf]]; Meshtastic: [[meshtastic]]; app peers: [[delta-chat-securejoin]]
