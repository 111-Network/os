---
title: Preferred Networks
category: concept
summary: Ordered list of preferred networks; v1 only shapes UI and proposals.
tags: [mesh, bridge, networking]
aliases: [network order, preferred network list]
sources:
  - https://github.com/111-Network/os/blob/main/README.md
relations:
  relates_to: ["[[lxmf]]", "[[meshtastic]]", "[[meshcore]]", "[[delta-chat-securejoin]]"]
---

Ordered list of preferred networks; v1 only shapes UI and proposals.

## What it is

The root README describes a short, ordered list of preferred networks, curated in the Knowledge Layer: the node joins
them in sequence, starts heartbeats and data exchange on the first one found, and falls back through the list.
Layer 6 stores the list as the `[order]` table of `interfaces.toml`, using network ids from the foreign message model.

## Use when

- Deciding the default order the Messages rail, Settings, and the agent's setup proposals present networks in.
- Explaining which networks a node reaches and in what sequence a user would want them tried.

## Do NOT use when

- Runtime behavior is needed: joining in sequence, heartbeats, and fall-back belong to Layer 7's presence and
  gateway work.
- A network is not enabled: the order lists preferences, and an off network stays off.

## Details

Default order (Layer 6 spec section 11):

1. `111`: the default fabric, always first.
2. `lxmf`: Reticulum, reached through the LXMF helper; Reticulum has no entry of its own in the order, see [[lxmf]].
3. `meshtastic`: the board 111 drives through the multiplexer ([[meshtastic]]).
4. `app:deltachat`, then `app:simplex`: app peers, which also decrypt on the hub ([[delta-chat-securejoin]]).

In v1 the order drives the rail, Settings, and agent proposals only. Until Layer 7 lands, the node is on every enabled
network at once. The user can reorder; the KB supplies the default and the reasoning. MeshCore is a transport option
([[meshcore]]) and not an entry of the default list.

## Evidence

- Source of the idea: the root README ("Existing mesh networks are treated as pure transport").
- Order and ids from the Layer 6 spec sections 3 and 11; the runtime half is explicitly deferred to Layer 7.

## Links

- Reticulum carrier: [[lxmf]]; Meshtastic: [[meshtastic]]; app peers: [[delta-chat-securejoin]]
