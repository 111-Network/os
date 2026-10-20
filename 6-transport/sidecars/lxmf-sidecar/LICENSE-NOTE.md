# Why this directory has no licence file yet

The helper's licence is an **open item of D27**: it is pending the owner's contact with the Reticulum project and a legal
read (spec `os/6-transport/README.md` §15 "Licensing boundary"; `LICENSING.md` §8). Until then:

- no `LICENSE` file is shipped here and the source file carries no SPDX identifier, so no licence is granted or implied by
  the repository for this directory; the other layers' licences (`LICENSE.md`) do not extend to it by omission;
- the helper is a **separate process** in its own venv. It is never linked into or combined with `111d`, and no Rust crate
  depends on it or on Reticulum;
- it imports the **user-installed, unmodified** `rns` and `lxmf` packages. They are downloaded hash-locked from PyPI at install
  time after a licence notice that the user acknowledges (D26 #5); no Reticulum code, documentation or data is copied into this
  repository, and the tests use a fake surface written for them;
- Reticulum's authorship is credited in the source header, `README.md`, `THIRD-PARTY-NOTICES.md` and the Settings copy:
  "Reticulum and LXMF are the work of the Reticulum project (reticulum.network); 111 uses them as released and is not
  affiliated";
- how the Reticulum License (non-OSI; no-harm and no-AI-training clauses) sits with this helper's own licence follows that
  contact. The open choices are a permissive licence or GPL-3.0-or-later with an explicit note that the user installs `rns`
  and `lxmf` themselves.

When the owner settles it, this file is replaced by the licence text and the SPDX header lands in `lxmf_sidecar.py`,
`gen_lock.py` and the tests in one change, together with `LICENSING.md` §1/§7/§8.
