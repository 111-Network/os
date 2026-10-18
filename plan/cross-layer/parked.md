# Parked — owner notes for later layers

Ideas the owner decided on 2026-10-14 (D17) but did **not** want built yet. Each entry says what, why it waits,
which layer picks it up, and what it needs. Keep entries short; move an entry into a layer brief when that layer starts.

| # | What | Waits for | Picked up by | Notes |
|---|---|---|---|---|
| P1 | **Meshtastic bridge** — relay messages between a Meshtastic network and ours | Messaging (Layer 5) and the Layer 6 device core | Layer 6/7 | Until then Meshtastic boards are detected and named only (D17 #2). Read-only presence into Nearby is the first step when it comes. |
| P2 | **Agent installs and tests existing apps on a connected phone** — once the agent reaches a phone (Termux), it can install apps such as Briar, test their connectivity, make the phone a node on those networks too, and bridge them with ours | Phone relay v1 (Layer 6) + Messaging (Layer 5) | Layer 6/7 | Owner's words: "once connected to a phone, we can access it and download existing apps like Briar etc and test connection with those as well and become a node there as well and bridge within ours with others". Fully agentic: no per-app hardcoding; the agent reads the app's docs from the KB. Legal/ToS read per app before automation. |
| P3 | **Analog radios (Baofeng-style handhelds)** — the worked example of a non-digital device | Messaging (Layer 5); licensing rules per country | A later layer (the owner will say which) | Detect + explain is already in the device registry (`analog`). Anything transmitted is licensed and plain-text by law; never encrypted. Keep as the example for "anything the user owns can be a device". |
| P4 | **Apply → verify → undo-on-failure engine** for device actions ("apply, verify, undo is smart!") | The typed-action vocabulary of Layer 6 | Layer 9 (self-healing), or a narrow cut in Layer 6 if the core cannot ship without it | Owner wants it, not now. Every action verified, auto-reverted on failure; node-child restarts may skip approval but are rate-limited (D17 #7). |
| P5 | **Own light mesh core in Rust** — if our own mesh transport ever depends on Reticulum code, rewrite it as ours | Layer 6/7 design | Layer 6/7 | Reticulum stays a connected service, used as-is and separately (D17 #12). The protocol design is a public-domain reference; its code is never incorporated (`os/LICENSING.md`). |
| P6 | **rnsd posture as a user setting** | Layer 6 settings page | Layer 6 | Default fixed by D17 #5; the setting UI lives on Settings → Layer 6. |
| P7 | **MeshChatX panel** — Reticulum/LXMF messaging inside the dashboard (0BSD+MIT fork of MeshChat, Reticulum-manual-listed; headless, `--no-https`, `--auth`, proxy-aware) | The panel proxy (Layer 5 build) + owner go | Layer 5 follow-up or Layer 6/7 | Owner chose Meshtastic as the only hosted panel this cycle (D19 f). Needs `share_instance = yes` (D17 #5), its own venv with `rns` from PyPI (separate adopted program; add a line to `LICENSING.md`), provenance check of the fork. Original MeshChat (MIT, no auth) is the fallback. |

Open hardware gates (not ideas, but also waiting): on-device phone run (`10-bootstrap/verify/README.md` V1: run-as env,
pair link, `.local` on Android) · real-Android check of the LAN-IP pair link · keyboard-aware Agent view
(`interactive-widget=resizes-content`) on a real phone.
