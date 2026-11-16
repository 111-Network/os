//! Layer 8 (§9.4; §16.1): T0 stub; filled by row D43.
//!
//! `encyclopedia.search`'s route, `GET /api/v1/datasets/search` (a session or loopback; the agent's
//! bearer reads): while kiwix-serve runs, `111d` asks its documented `/search` endpoint on
//! `127.0.0.1:11250` and answers `{schema, format: "opensearch-xml", body}`, the upstream text
//! capped at 8 KiB and parsed by nothing; `book=<id>` maps to the `book` field of the entry the
//! publisher holds under that id; each search resets the idle counter; while it is off, `409
//! component_unavailable` ("The encyclopedia is off."). It never starts kiwix-serve.
#![allow(dead_code)]

/// One search: `q`, `book`, `publisher`, `n` (§9.4). T0: the name only.
pub struct SearchQuery;
