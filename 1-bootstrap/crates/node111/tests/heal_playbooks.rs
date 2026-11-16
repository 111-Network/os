//! Layer 9 playbooks (`9-evolution/README.md` §3.4, §6): the one vocabulary (row D3); rows D4 and
//! D5 add the grammar, the loader and the T0 evaluator over `tests/fixtures/playbooks/`.
//!
//! Row D3 (§3.4 "One subject id space, one source per failure"; §6.3 "One vocabulary, one source";
//! G25 (5)'s drift half), the tests named `vocabulary_*`
//! (`cargo test -p node111 --test heal_playbooks vocabulary`):
//! - the literal scan: every reason literal a row constructor of `src/daemon/services/mod.rs`
//!   publishes in state `error`, in its own body or in a helper it calls (`child_rule`), is a row
//!   of `daemon::heal::vocab::ROWS` sourced by that service row, unless it is a child code; every
//!   row a service row sources is still published; a code in `error` that is not a literal, or
//!   that no row constructor reaches, fails the scan instead of escaping it;
//! - a state's way in: a row's state and code reach `Computed::new` as a `ServiceState::` variant
//!   and its literal, or whole from one fn of that file; a state the scan cannot follow back to a
//!   literal (another module's rule, a facts type's method, a code remapped or set after the call,
//!   an alias of `ServiceState`, a `Computed` built in another file) fails it the same way;
//! - the rows no service row sources are §3.4's (`child_exit` for each supervised service and for
//!   `any` component; `111d`, `updater`, `storage`, `heal`), each subject at its layer; the child
//!   codes, the `unset` codes and the `gateway` and `codeagent` rows are no row;
//! - the committed `0-knowledge/_meta/playbook-vocabulary.toml`, which `build.py` reads, equals the
//!   table's rendering byte for byte (`UPDATE_PLAYBOOK_VOCABULARY=1` rewrites it; CI runs this
//!   test, so CI fails on drift);
//! - a literal missing from the table fails, and so does a state the scan cannot read: the scan
//!   run over mutated copies of the source.

use node111::daemon::heal::vocab::{self, VocabRow, ANY, CHILD_EXIT, ROWS};
use node111::daemon::services::{catalogue, Kind, ServiceId};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- the vocabulary (row D3)

/// The codes a row publishes in `error` that open nothing (§3.4): the child's state watch and exit
/// facts are the source (`spawn_pending` is `starting` today; listed as the spec lists it).
const CHILD_CODES: &[&str] = &[
    "child_backoff",
    "child_degraded",
    "child_stopped",
    "spawn_pending",
    "transport_degraded",
];

/// Set to `1`, the file test rewrites the committed vocabulary file instead of comparing it.
const UPDATE_ENV: &str = "UPDATE_PLAYBOOK_VOCABULARY";

/// How the row constructors write the state: `Computed::new(id, ServiceState::Error, "code", …)`
/// and `child_rule`'s `(ServiceState::Error, "code", reason)`.
const ERROR_STATE: &str = "ServiceState::Error";

/// The one way a row gets its state and its code: `Computed::new(id, state, code, reason)`.
const COMPUTED_NEW: &str = "Computed::new(";

/// The start of the module's tests, which the scan leaves out (their asserts name codes too).
const TESTS_START: &str = "\n#[cfg(test)]\nmod tests";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn services_src() -> String {
    let p = manifest_dir().join("src/daemon/services/mod.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// Every `.rs` file under the crate's `src/` but `src/daemon/services/mod.rs`, by its path from
/// the crate root, sorted.
fn crate_sources() -> Vec<(String, String)> {
    let root = manifest_dir();
    let mut out = Vec::new();
    let mut dirs = vec![root.join("src")];
    while let Some(dir) = dirs.pop() {
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
                .path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "rs")
                && !path.ends_with("src/daemon/services/mod.rs")
            {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                let rel = path.strip_prefix(root).unwrap_or(&path);
                out.push((rel.display().to_string(), text));
            }
        }
    }
    out.sort();
    out
}

/// `0-knowledge/_meta/playbook-vocabulary.toml`, from the repository root (`node111` is
/// `1-bootstrap/crates/node111`).
fn vocabulary_path() -> PathBuf {
    let root = manifest_dir()
        .ancestors()
        .nth(3)
        .expect("the crate sits three levels under the repository root");
    root.join("0-knowledge/_meta/playbook-vocabulary.toml")
}

/// §6.2: lower case, digits and `_`.
fn is_code(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The start of the identifier that ends at `end` in `text`.
fn ident_start(text: &str, end: usize) -> usize {
    text[..end]
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_ident(c))
        .map_or(0, |(k, c)| k + c.len_utf8())
}

/// Where `word` stands alone in `text`: no identifier character on either side.
fn word_indices<'t>(text: &'t str, word: &'t str) -> impl Iterator<Item = usize> + 't {
    text.match_indices(word).map(|(i, _)| i).filter(move |&i| {
        !text[..i].ends_with(is_ident) && !text[i + word.len()..].starts_with(is_ident)
    })
}

/// `text` with every comment and the inside of every string and char literal blanked to spaces,
/// byte for byte (quotes and line breaks stay): offsets agree with `text`, and a search of it sees
/// code only, never a doc line, a sentence or a trailing comment.
fn mask(text: &str) -> String {
    fn blank(out: &mut [u8], from: usize, to: usize) {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        i = if b[i..].starts_with(b"//") {
            let end = text[i..].find('\n').map_or(b.len(), |e| i + e);
            blank(&mut out, i, end);
            end
        } else if b[i..].starts_with(b"/*") {
            let end = text[i + 2..].find("*/").map_or(b.len(), |e| i + e + 4);
            blank(&mut out, i, end);
            end
        } else if b[i] == b'"' {
            // a raw string (`r"…"`, `r#"…"#`) ends at `"` and its `#`s; any other string at the
            // first unescaped `"`
            let hashes = b[..i].iter().rev().take_while(|&&c| c == b'#').count();
            let (end, next) = if i > hashes && b[i - hashes - 1] == b'r' {
                let close = format!("\"{}", "#".repeat(hashes));
                let end = text[i + 1..].find(&close).map_or(b.len(), |e| i + 1 + e);
                (end, end + close.len())
            } else {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                (j.min(b.len()), j + 1)
            };
            blank(&mut out, i + 1, end);
            next
        } else if b[i] == b'\'' {
            // a char literal (`'x'`, `'"'`, `'\n'`) is blanked; a lifetime (`'static`) stays
            let end = if b.get(i + 1) == Some(&b'\\') {
                text.get(i + 3..)
                    .and_then(|t| t.find('\''))
                    .map(|e| i + 3 + e)
            } else {
                text[i + 1..]
                    .chars()
                    .next()
                    .map(|c| i + 1 + c.len_utf8())
                    .filter(|&e| b.get(e) == Some(&b'\''))
            };
            match end {
                Some(e) => {
                    blank(&mut out, i + 1, e);
                    e + 1
                }
                None => i + 1,
            }
        } else {
            i + 1
        };
    }
    String::from_utf8(out).expect("whole characters only are blanked")
}

/// One top-level `fn` of the scanned code: its name, where it starts, and its text through its
/// closing line.
struct Item<'a> {
    name: &'a str,
    at: usize,
    text: &'a str,
}

/// The name of the top-level `fn` a line declares (`fn x(`, `pub fn x(`, `pub(crate) const fn x(`).
fn fn_name(line: &str) -> Option<&str> {
    let mut rest = line;
    for vis in ["pub(crate) ", "pub(super) ", "pub "] {
        if let Some(r) = rest.strip_prefix(vis) {
            rest = r;
            break;
        }
    }
    for qualifier in ["const ", "async ", "unsafe "] {
        if let Some(r) = rest.strip_prefix(qualifier) {
            rest = r;
        }
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !is_ident(c))?;
    Some(&rest[..end]).filter(|n| !n.is_empty())
}

/// The top-level `fn` items of `src`, as rustfmt lays them out: the item starts at column 0 and
/// ends at the first line that is a lone `}` (or on its own line when it is one line long).
fn top_level_fns(src: &str) -> Result<Vec<Item<'_>>, String> {
    let mut items = Vec::new();
    let mut open: Option<(&str, usize)> = None;
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let text = line.trim_end();
        match open {
            None => {
                if let Some(name) = fn_name(text) {
                    if text.ends_with('}') {
                        items.push(Item {
                            name,
                            at: start,
                            text: &src[start..at],
                        });
                    } else {
                        open = Some((name, start));
                    }
                }
            }
            Some((name, from)) => {
                if text == "}" {
                    items.push(Item {
                        name,
                        at: from,
                        text: &src[from..at],
                    });
                    open = None;
                }
            }
        }
    }
    match open {
        Some((name, _)) => Err(format!("fn {name} has no closing `}}` line")),
        None => Ok(items),
    }
}

/// The reason literals the masked `text` of an item publishes in state `error` (`raw`: the same
/// bytes unmasked). Each `ServiceState::Error` must be followed by `,` and a string literal:
/// anything else (a code in a variable, a struct literal, a pattern) is reported, never skipped,
/// so no code escapes the scan.
fn error_literals(raw: &str, text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices(ERROR_STATE) {
        let after = &text[i + ERROR_STATE.len()..];
        if after.starts_with(is_ident) {
            continue; // a longer identifier
        }
        let literal = after
            .trim_start()
            .strip_prefix(',')
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix('"'))
            .and_then(|r| {
                let at = text.len() - r.len();
                r.find('"').map(|end| &raw[at..at + end])
            });
        match literal {
            Some(code) if is_code(code) => out.push(code.to_string()),
            _ => {
                let at = text.len() - after.len();
                let shown: String = raw[at..].trim_start().chars().take(48).collect();
                return Err(format!(
                    "`{ERROR_STATE}` not followed by a reason literal: `{shown}`"
                ));
            }
        }
    }
    Ok(out)
}

/// Whether `text` calls the top-level fn `name`: `name(` not preceded by an identifier
/// character, `.` (a method) or `:` (a path).
fn calls(text: &str, name: &str) -> bool {
    let pattern = format!("{name}(");
    text.match_indices(&pattern).any(|(i, _)| {
        text[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(is_ident(c) || c == '.' || c == ':'))
    })
}

/// The arguments of the call or tuple whose `(` is at `open` in masked `text`: their spans, split
/// at its top-level commas (a trailing comma dropped), and the index of its `)`.
fn args_at(text: &str, open: usize) -> Option<(Vec<Range<usize>>, usize)> {
    let mut depth = 0usize;
    let mut from = open + 1;
    let mut out = Vec::new();
    for (j, c) in text.bytes().enumerate().skip(open + 1) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth > 0 => depth -= 1,
            b')' => {
                if !text[from..j].trim().is_empty() {
                    out.push(from..j);
                }
                return Some((out, j));
            }
            b',' if depth == 0 => {
                out.push(from..j);
                from = j + 1;
            }
            _ => {}
        }
    }
    None
}

/// Whether `rest`, what follows a place, starts with a lone `=`: an assignment, not `==` or `=>`.
fn is_assignment(rest: &str) -> bool {
    let r = rest.trim_start();
    r.starts_with('=') && !r.starts_with("==") && !r.starts_with("=>")
}

/// How many `let` patterns of masked `text` bind `name`, and whether `text` assigns it (`name =`
/// outside a pattern): a name one `let` binds and nothing assigns holds that `let`'s value.
fn bindings(text: &str, name: &str) -> (usize, bool) {
    let mut patterns = Vec::new();
    for i in word_indices(text, "let") {
        let from = i + "let".len();
        // the pattern ends at its `=` (not `==`, `=>`, `<=`, `>=`, `!=`) or, with no value, its `;`
        let end = text[from..]
            .char_indices()
            .find(|&(k, c)| {
                c == ';'
                    || (c == '='
                        && is_assignment(&text[from + k..])
                        && !text[..from + k].ends_with(['=', '!', '<', '>']))
            })
            .map_or(text.len(), |(k, _)| from + k);
        patterns.push(from..end);
    }
    let lets = patterns
        .iter()
        .filter(|p| word_indices(&text[(*p).clone()], name).next().is_some())
        .count();
    let assigned = word_indices(text, name).any(|i| {
        !patterns.iter().any(|p| p.contains(&i))
            && !text[..i].ends_with('.')
            && is_assignment(&text[i + name.len()..])
    });
    (lets, assigned)
}

/// The `let (first, b, ..) = callee(..)` of masked `text` whose first name is `first`: its names
/// (`mut` dropped) and its callee as written (`child_rule`, `rules::child_rule`,
/// `g.state_and_code`; empty when the value is no call).
fn destructuring<'t>(text: &'t str, first: &str) -> Option<(Vec<&'t str>, &'t str)> {
    word_indices(text, "let").find_map(|i| {
        if !text[i..].starts_with("let (") {
            return None;
        }
        let (spans, close) = args_at(text, i + "let ".len())?;
        let names: Vec<&str> = spans
            .into_iter()
            .map(|r| {
                let n = text[r].trim();
                n.strip_prefix("mut ").unwrap_or(n).trim()
            })
            .collect();
        if names.first() != Some(&first) {
            return None;
        }
        let value = text[close + 1..]
            .trim_start()
            .strip_prefix('=')?
            .trim_start();
        let len = value
            .find(|c: char| !(is_ident(c) || c == ':' || c == '.'))
            .unwrap_or(value.len());
        let callee = if value[len..].starts_with('(') {
            &value[..len]
        } else {
            ""
        };
        Some((names, callee))
    })
}

/// R1, a state's way in: in each `Computed::new(id, state, code, reason)` of `it`, `state` is a
/// `ServiceState::` variant (an `Error` then followed by its literal, [`error_literals`]), or
/// `state` and `code` are the first two names of one `let (state, code, ..) = helper(..)` of the
/// fn, `helper` a fn of this file and each name bound by that `let` alone and never assigned. A
/// state from anywhere else (another module's rule, a facts type's method, a code remapped after
/// the call) publishes codes the scan cannot read: a problem, never a skip. Each `helper` goes on
/// to [`helper_calls`].
fn state_args<'a>(
    it: &Item<'a>,
    raw: &str,
    fns: &BTreeSet<&str>,
    helpers: &mut BTreeSet<&'a str>,
    problems: &mut Vec<String>,
) {
    let text = it.text;
    let shown = |r: &Range<usize>| {
        raw[r.clone()]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    for i in word_indices(text, "Computed").filter(|&i| text[i..].starts_with(COMPUTED_NEW)) {
        let Some((args, _)) =
            args_at(text, i + COMPUTED_NEW.len() - 1).filter(|(a, _)| a.len() == 4)
        else {
            problems.push(format!(
                "fn {}: a `Computed::new` the scan cannot split into its four arguments",
                it.name
            ));
            continue;
        };
        let (state, code) = (text[args[1].clone()].trim(), text[args[2].clone()].trim());
        if state
            .strip_prefix("ServiceState::")
            .is_some_and(|v| !v.is_empty() && v.chars().all(is_ident))
        {
            continue;
        }
        let why = if state.is_empty() || !state.chars().all(is_ident) {
            "it is no `ServiceState::` variant and no name".to_string()
        } else {
            match destructuring(text, state) {
                None => format!("no `let ({state}, ..)` binds it"),
                Some((names, _)) if names.get(1) != Some(&code) => {
                    format!("its code is not the second name of the `let` that binds `{state}`")
                }
                Some((_, "")) => format!("the `let` that binds `{state}` calls no fn"),
                Some((_, callee)) if !fns.contains(callee) => {
                    format!("`{callee}` is not a fn of this file")
                }
                Some((_, callee)) => {
                    match [state, code]
                        .into_iter()
                        .find(|n| bindings(text, n) != (1, false))
                    {
                        Some(n) => format!("`{n}` is bound again or assigned"),
                        None => {
                            helpers.insert(callee);
                            continue;
                        }
                    }
                }
            }
        };
        problems.push(format!(
            "fn {}: `Computed::new` takes the state `{}` and the code `{}`: {why}; the scan \
             follows a state only from a `ServiceState::` variant or, whole, from one fn of this \
             file, so it cannot read the codes this publishes",
            it.name,
            shown(&args[1]),
            shown(&args[2])
        ));
    }
}

/// The calls in masked `text` of a fn by its plain name (`f(`) or by a module path (`m::f(`), as
/// written; a method (`.f(`), a type's own fn (`T::f(`), a macro (`f!(`), a constructor
/// (`Some(`), a `fn(` type and the item's own `fn` line are none.
fn calls_in(text: &str) -> Vec<&str> {
    let lower = |s: &str| s.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
    let mut out = Vec::new();
    for (j, _) in text.match_indices('(') {
        let start = ident_start(text, j);
        let name = &text[start..j];
        if !lower(name) || ["fn", "if", "match", "while", "return"].contains(&name) {
            continue;
        }
        let mut from = start;
        while text[..from].ends_with("::") {
            from = ident_start(text, from - 2);
        }
        let path = &text[from..j];
        let lead = &text[..from];
        if lead.ends_with('.')
            || lead.ends_with("fn ")
            || path.rsplit("::").nth(1).is_some_and(|owner| !lower(owner))
        {
            continue;
        }
        out.push(path);
    }
    out
}

/// R2, a helper's tuples: a fn whose result is a row's state ([`state_args`]) calls, by plain
/// name or module path, only fns of this file, and those the same, so no rule of another module
/// hands it a state. A method or a type's own fn is not followed (`child_rule` calls `is_none`):
/// a state helper that reached a `ServiceState` through one is the form this scan cannot see.
fn helper_calls<'a>(items: &[Item<'a>], helpers: BTreeSet<&'a str>, problems: &mut Vec<String>) {
    let mut todo: Vec<&str> = helpers.into_iter().collect();
    let mut seen = BTreeSet::new();
    while let Some(name) = todo.pop() {
        let Some(it) = items.iter().find(|it| it.name == name) else {
            continue;
        };
        if !seen.insert(name) {
            continue;
        }
        for callee in calls_in(it.text) {
            if items.iter().any(|x| x.name == callee) {
                todo.push(callee);
            } else {
                problems.push(format!(
                    "fn {name}, whose result is a row's state, calls `{callee}`, not a fn of this \
                     file: the scan cannot read the states it hands on"
                ));
            }
        }
    }
}

/// The places masked `text` builds a `Computed` by struct literal: `Computed {` (a path before it
/// allowed) where a value goes, and `Self {` inside an `impl … Computed`; a declaration
/// (`struct`, `enum`, `impl`, `for`, `-> Computed {`) or a `let` pattern is none.
fn computed_literals(text: &str) -> usize {
    let builds = |at: usize| {
        let mut k = at;
        while text[..k].ends_with("::") {
            k = ident_start(text, k - 2);
        }
        let before = text[..k].trim_end();
        let word = &before[ident_start(before, before.len())..];
        !before.ends_with("->") && !["struct", "enum", "impl", "for", "let"].contains(&word)
    };
    let mut n = word_indices(text, "Computed")
        .filter(|&i| text[i + "Computed".len()..].trim_start().starts_with('{') && builds(i))
        .count();
    for i in word_indices(text, "impl") {
        let header = text[i..].split('{').next().unwrap_or_default().trim_end();
        if &header[ident_start(header, header.len())..] != "Computed" {
            continue;
        }
        let body = &text[i..text[i..].find("\n}").map_or(text.len(), |e| i + e)];
        n += word_indices(body, "Self")
            .filter(|&j| {
                body[j + "Self".len()..].trim_start().starts_with('{')
                    && !body[..j].trim_end().ends_with("->")
            })
            .count();
    }
    n
}

/// R3, the side doors of the scanned code: a row's state and code are set by `Computed::new`
/// alone, so no `reason_code` field is assigned (`c.reason_code = "…"`), nor a `state` field a
/// `ServiceState` may reach (`c.state = s`; a value of another type, such as `PanelState::Live`
/// or `Some(…)` for the file's other two `state` fields, is none), no `Computed` is built but by
/// `Computed::new`'s one struct literal, and no `use` or `type` aliases `ServiceState` (`use
/// ServiceState::*;` lets `Error` stand for the variant unread).
fn side_doors(code: &str, problems: &mut Vec<String>) {
    // a value that cannot be a `ServiceState`: another type's path or constructor, or a string
    let other_type = |value: &str| {
        let v = value.trim_start().trim_start_matches('=').trim_start();
        (v.starts_with(|c: char| c.is_ascii_uppercase()) && !v.starts_with("ServiceState"))
            || v.starts_with('"')
    };
    for field in ["state", "reason_code"] {
        let set = word_indices(code, field)
            .filter(|&i| {
                let rest = &code[i + field.len()..];
                code[..i].ends_with('.')
                    && is_assignment(rest)
                    && !(field == "state" && other_type(rest))
            })
            .count();
        if set > 0 {
            problems.push(format!(
                "{set} assignment(s) to a `.{field}` field: a row's state and code are set by \
                 `Computed::new` alone, where the scan reads them"
            ));
        }
    }
    let literals = computed_literals(code);
    if literals != 1 {
        problems.push(format!(
            "{literals} `Computed` struct literals: the scan reads rows built by \
             `Computed::new`, whose literal is the one it allows"
        ));
    }
    for i in word_indices(code, "use").chain(word_indices(code, "type")) {
        let stmt = code[i..].split(';').next().unwrap_or_default();
        let alias = match stmt.split_once('=') {
            Some((_, of)) if stmt.starts_with("type") => of.trim() == "ServiceState",
            _ => stmt.starts_with("use") && word_indices(stmt, "ServiceState").next().is_some(),
        };
        if alias {
            problems.push(format!(
                "`{}`: an alias of `ServiceState` hides a state from the scan",
                stmt.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
}

/// R4: no file of `src/` but `services/mod.rs` builds a `Computed` (a `Computed::new` call or a
/// struct literal), so no row constructor hands back a row built where the scan does not read.
/// `files`: paths from the crate root with their text; each module's trailing tests are left out.
fn computed_built_elsewhere(files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .filter_map(|(path, src)| {
            let code = mask(
                src.find(TESTS_START)
                    .map_or(src.as_str(), |cut| &src[..cut]),
            );
            let n = word_indices(&code, "Computed")
                .filter(|&i| code[i..].starts_with(COMPUTED_NEW))
                .count()
                + computed_literals(&code);
            (n > 0).then(|| {
                format!(
                    "{path} builds a `Computed` in {n} place(s): only the row constructors of \
                     src/daemon/services/mod.rs may, where the scan reads the codes they publish"
                )
            })
        })
        .collect()
}

/// The scan: each `(service row, reason literal)` a row constructor of `services/mod.rs` publishes
/// in state `error`. The row constructor of a service is `fn <service id>(f: &Facts)`, called by
/// `compute`; a literal in a helper counts for every row constructor that reaches it. It reads the
/// code with its comments and literal text blanked ([`mask`]) and fails, never skips, where it
/// cannot follow a row's state back to a literal: R1 [`state_args`], R2 [`helper_calls`], R3
/// [`side_doors`] (R4, the rest of `src/`, is [`computed_built_elsewhere`]).
fn scan(src: &str) -> Result<BTreeSet<(&'static str, String)>, Vec<String>> {
    let Some(cut) = src.find(TESTS_START) else {
        return Err(vec![
            "the scan expects the module's tests last, under `#[cfg(test)] mod tests`".into(),
        ]);
    };
    let raw = &src[..cut];
    let masked = mask(raw);
    let code = masked.as_str();
    let items = top_level_fns(code).map_err(|e| vec![e])?;
    let fns: BTreeSet<&str> = items.iter().map(|it| it.name).collect();
    let mut problems = Vec::new();
    for needle in [ERROR_STATE, COMPUTED_NEW] {
        let inside: usize = items.iter().map(|it| it.text.matches(needle).count()).sum();
        if inside != code.matches(needle).count() {
            problems.push(format!(
                "a `{needle}` outside a top-level fn: the scan cannot tell which row publishes it"
            ));
        }
    }
    side_doors(code, &mut problems);
    let mut literals: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut helpers = BTreeSet::new();
    for it in &items {
        let unmasked = &raw[it.at..it.at + it.text.len()];
        match error_literals(unmasked, it.text) {
            Ok(found) if found.is_empty() => {}
            Ok(found) => {
                literals.insert(it.name, found);
            }
            Err(e) => problems.push(format!("fn {}: {e}", it.name)),
        }
        state_args(it, unmasked, &fns, &mut helpers, &mut problems);
    }
    helper_calls(&items, helpers, &mut problems);
    let compute = items.iter().find(|it| it.name == "compute");
    let mut reached: BTreeSet<&str> = BTreeSet::new();
    let mut published = BTreeSet::new();
    for id in ServiceId::ALL {
        let name = id.as_str();
        let called = compute.is_some_and(|c| c.text.contains(&format!("{name}(f)")));
        if !called || !items.iter().any(|it| it.name == name) {
            problems.push(format!(
                "no row constructor `fn {name}(f: &Facts)` called by `compute`: the scan cannot \
                 read the `{name}` row's codes"
            ));
            continue;
        }
        let mut seen = BTreeSet::from([name]);
        let mut stack = vec![name];
        while let Some(f) = stack.pop() {
            let Some(body) = items.iter().find(|it| it.name == f) else {
                continue;
            };
            for it in &items {
                if !seen.contains(it.name) && calls(body.text, it.name) {
                    seen.insert(it.name);
                    stack.push(it.name);
                }
            }
        }
        for f in seen {
            reached.insert(f);
            for code in literals.get(f).into_iter().flatten() {
                published.insert((name, code.clone()));
            }
        }
    }
    for (f, found) in &literals {
        if !reached.contains(f) {
            problems.push(format!(
                "fn {f} publishes {found:?} in state `error` but no row constructor calls it"
            ));
        }
    }
    if problems.is_empty() {
        Ok(published)
    } else {
        Err(problems)
    }
}

/// The table against a scan: a published literal the table lacks for its row, a row no
/// constructor publishes any more, a child code in the table.
fn vocabulary_problems(published: &BTreeSet<(&'static str, String)>) -> Vec<String> {
    let mut problems = Vec::new();
    for (id, code) in published {
        if CHILD_CODES.contains(&code.as_str()) {
            continue;
        }
        let service = ServiceId::parse(id).expect("the scan names service ids");
        if vocab::of_service_row(service, code).is_none() {
            problems.push(format!(
                "the `{id}` row publishes `{code}` in state `error` and the vocabulary has no row \
                 for it: add its row to ROWS in src/daemon/heal/vocab.rs and run with \
                 {UPDATE_ENV}=1 (a child code goes to CHILD_CODES in this test instead)"
            ));
        }
    }
    for r in ROWS {
        if let Some(id) = r.service_row {
            if !published.contains(&(id.as_str(), r.reason.to_string())) {
                problems.push(format!(
                    "stale row ({}, {}, {}): the `{}` row no longer publishes `{}` in state `error`",
                    r.layer,
                    r.subject,
                    r.reason,
                    id.as_str(),
                    r.reason
                ));
            }
        }
        if CHILD_CODES.contains(&r.reason) {
            problems.push(format!("a child code is a row: {r:?}"));
        }
    }
    problems
}

/// The file `build.py` reads: one `[[row]]` per entry of the table, sorted by `(layer, subject,
/// reason)`; `service_row` only where a service row is the source.
fn render(rows: &[VocabRow]) -> String {
    let mut sorted: Vec<&VocabRow> = rows.iter().collect();
    sorted.sort_by_key(|r| (r.layer, r.subject, r.reason));
    let mut out = String::from(
        "# node111's compiled (layer, subject, reason) vocabulary (9-evolution/README.md §3.4, §6.3):\n\
         # one [[row]] per fingerprint a playbook's [match] may name, sorted by layer, subject and\n\
         # reason in byte order. Subject \"any\" is every component. service_row names the service row\n\
         # whose state `error` publishes the reason; it is absent for an exit fact (child_exit), a\n\
         # start fact (111d, updater), Layer 8's storage state (storage) and Layer 9's own (heal).\n\
         # Generated from 1-bootstrap/crates/node111/src/daemon/heal/vocab.rs by\n\
         # `cargo test -p node111 --test heal_playbooks vocabulary`; do not edit it by hand:\n\
         # UPDATE_PLAYBOOK_VOCABULARY=1 rewrites it, and the test, in CI too, fails while it differs.\n\
         # The playbook lint (0-knowledge/_meta/build.py) reads it.\n\
         schema = 1\n",
    );
    for r in sorted {
        assert!(
            is_code(r.subject) && is_code(r.reason),
            "a token TOML would need to escape: {r:?}"
        );
        out.push_str(&format!(
            "\n[[row]]\nlayer = {}\nsubject = \"{}\"\nreason = \"{}\"\n",
            r.layer, r.subject, r.reason
        ));
        if let Some(id) = r.service_row {
            out.push_str(&format!("service_row = \"{}\"\n", id.as_str()));
        }
    }
    out
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VocabularyFile {
    schema: u32,
    row: Vec<FileRow>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct FileRow {
    layer: u8,
    subject: String,
    reason: String,
    service_row: Option<String>,
}

/// §3.4's table: the layer of each subject.
fn spec_layer(subject: &str) -> u8 {
    match subject {
        "kb" => 0,
        "111d" | "updater" => 1,
        "identity" => 2,
        "agent" | "model" => 3,
        "fabric" | "meshtastic" => 5,
        "transport" | "radio" | ANY => 6,
        "storage" => 8,
        "heal" => 9,
        other => panic!("`{other}` is no subject of §3.4"),
    }
}

#[test]
fn vocabulary_holds_every_error_literal_the_rows_publish() {
    let published = scan(&services_src())
        .unwrap_or_else(|p| panic!("the literal scan failed:\n{}", p.join("\n")));
    // the scan reads every constructor, `child_rule` through the two rows that call it
    for (id, code) in [
        ("kb", "verify_failed"),
        ("agent", "health_503"),
        ("model", "health_unreachable"),
        ("model", "child_degraded"),
        ("transport", "transport_degraded"),
        ("radio", "stack_down"),
        ("fabric", "no_identity"),
        ("fabric", "store_full"),
        ("meshtastic", "port_busy"),
    ] {
        assert!(
            published.contains(&(id, code.to_string())),
            "the scan missed ({id}, {code}): {published:?}"
        );
    }
    // only `error`: an `unset` or `live` code is never read
    for (id, code) in [
        ("kb", "kb_absent"),
        ("agent", "binary_absent"),
        ("fabric", "hub_reachable"),
    ] {
        assert!(
            !published.contains(&(id, code.to_string())),
            "({id}, {code})"
        );
    }
    assert!(
        !published
            .iter()
            .any(|(id, _)| *id == "gateway" || *id == "codeagent"),
        "the gateway and codeagent rows publish no `error` code: {published:?}"
    );
    let problems = vocabulary_problems(&published);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn vocabulary_rows_no_service_row_sources_are_the_spec_s() {
    for r in ROWS {
        assert_eq!(r.layer, spec_layer(r.subject), "{r:?}");
    }
    let mut want: BTreeSet<(u8, String, String)> = BTreeSet::new();
    let mut fixed = |subject: &str, reason: &str| {
        want.insert((spec_layer(subject), subject.to_string(), reason.to_string()));
    };
    // an unrequested exit of each supervised child (the catalogue's `child` rows) and of any
    // component (§3.2, §3.4 rule 2)
    for id in ServiceId::ALL {
        if catalogue(id).kind == Kind::Child {
            fixed(id.as_str(), CHILD_EXIT);
        }
    }
    fixed(ANY, CHILD_EXIT);
    // §3.3's start facts and rule 7, §12.6's APK check
    for reason in [
        "unclean_exit",
        "watchdog_kill",
        "unclean_boot",
        "killed_by_os",
        "witness_absent",
        "apk_outdated",
    ] {
        fixed("111d", reason);
    }
    fixed("updater", "update_reverted");
    fixed("updater", "release_reverted_at_start");
    // Layer 8's one source; Layer 9's own
    fixed("storage", "disk_low");
    fixed("heal", "revert_failed");
    fixed("heal", "store_unavailable");
    let got: BTreeSet<(u8, String, String)> = ROWS
        .iter()
        .filter(|r| r.service_row.is_none())
        .map(|r| (r.layer, r.subject.to_string(), r.reason.to_string()))
        .collect();
    assert_eq!(got, want);
    assert_eq!(
        want.iter()
            .filter(|(_, _, reason)| reason == CHILD_EXIT)
            .count(),
        5,
        "agent, model, transport, meshtastic and any"
    );

    // not rows (§3.4): the child codes, the `unset` codes, Layer 7's `gateway`, the `codeagent` row
    for r in ROWS {
        assert!(!CHILD_CODES.contains(&r.reason), "{r:?}");
        assert!(
            !["kb_absent", "carrier_pending"].contains(&r.reason),
            "{r:?}"
        );
        assert!(!["gateway", "codeagent"].contains(&r.subject), "{r:?}");
    }
    // the fabric row's identity codes are layer 2's, never a Layer 5 fingerprint (§3.4 row 2)
    for reason in ["no_identity", "restore_pending"] {
        assert_eq!(vocab::lookup("fabric", reason), None);
        let r = vocab::of_service_row(ServiceId::Fabric, reason).unwrap();
        assert_eq!((r.layer, r.subject), (2, "identity"));
    }
    // a full fabric store opens only (5, fabric, store_full) (R3)
    let full: Vec<&VocabRow> = ROWS.iter().filter(|r| r.reason == "store_full").collect();
    assert_eq!(full.len(), 1);
    assert_eq!((full[0].layer, full[0].subject), (5, "fabric"));
}

#[test]
fn vocabulary_file_equals_the_table() {
    for pair in ROWS.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert!(
            (a.layer, a.subject, a.reason) < (b.layer, b.subject, b.reason),
            "the table is not sorted or holds a row twice: {a:?} then {b:?}"
        );
    }
    let text = render(ROWS);
    // what build.py reads with tomllib is the table, row for row
    let parsed: VocabularyFile = toml::from_str(&text).expect("the rendering is TOML");
    assert_eq!(parsed.schema, 1);
    let want: Vec<FileRow> = ROWS
        .iter()
        .map(|r| FileRow {
            layer: r.layer,
            subject: r.subject.to_string(),
            reason: r.reason.to_string(),
            service_row: r.service_row.map(|id| id.as_str().to_string()),
        })
        .collect();
    assert_eq!(parsed.row, want);

    let path = vocabulary_path();
    if std::env::var(UPDATE_ENV).as_deref() == Ok("1") {
        std::fs::write(&path, &text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        eprintln!("note: {UPDATE_ENV}=1: wrote {}", path.display());
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} ({UPDATE_ENV}=1 writes it from the table)",
            path.display()
        )
    });
    assert!(
        committed == text,
        "{} differs from the compiled table in src/daemon/heal/vocab.rs ({UPDATE_ENV}=1 \
         rewrites it)\n--- committed\n{committed}\n--- the table\n{text}",
        path.display()
    );
}

#[test]
fn vocabulary_scan_fails_on_a_literal_missing_from_the_table() {
    let src = services_src();
    let cut = src.find(TESTS_START).expect("the module's tests");

    // (1) a row publishes in `error` a code the table lacks: the `kb` row's `kb_absent`, `unset` today
    let at = src[..cut]
        .find("\"kb_absent\"")
        .expect("the kb row's kb_absent");
    let unset = "ServiceState::Unset";
    let state = src[..at].rfind(unset).expect("its state");
    let mutated = format!(
        "{}{ERROR_STATE}{}",
        &src[..state],
        &src[state + unset.len()..]
    );
    let problems = vocabulary_problems(&scan(&mutated).expect("the mutated source scans"));
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("the `kb` row publishes `kb_absent`"),
        "{problems:?}"
    );

    // (2) a new literal in a helper reaches every row that calls it: `child_rule`'s, for both the
    // agent and the model; the code it replaced is stale for both
    let mutated = format!(
        "{}{}",
        src[..cut].replacen("\"health_503\"", "\"health_418\"", 1),
        &src[cut..]
    );
    assert!(mutated != src, "the test is stale: child_rule's health_503");
    let problems = vocabulary_problems(&scan(&mutated).expect("the mutated source scans"));
    assert_eq!(problems.len(), 4, "{problems:?}");
    for id in ["agent", "model"] {
        let missing = format!("the `{id}` row publishes `health_418`");
        assert!(
            problems.iter().any(|p| p.contains(&missing)),
            "{problems:?}"
        );
        let stale = format!("the `{id}` row no longer publishes `health_503`");
        assert!(problems.iter().any(|p| p.contains(&stale)), "{problems:?}");
    }

    // (3) a code in `error` that is not a literal fails the scan itself
    let mutated = format!(
        "{}{}",
        src[..cut].replacen("\"stack_down\"", "stack_code", 1),
        &src[cut..]
    );
    assert!(
        mutated != src,
        "the test is stale: the radio row's stack_down"
    );
    let problems = scan(&mutated).expect_err("a code that is not a literal");
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("fn radio:") && p.contains("stack_code")),
        "{problems:?}"
    );

    // (4) a literal no row constructor reaches fails the scan
    let mutated = format!(
        "{}\nfn orphan() -> (ServiceState, &'static str) {{\n    ({ERROR_STATE}, \"orphan_code\")\n}}\n{}",
        &src[..cut],
        &src[cut..]
    );
    let problems = scan(&mutated).expect_err("an unreached literal");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("fn orphan") && p.contains("orphan_code")),
        "{problems:?}"
    );

    // (5) a row constructor the scan cannot find fails it
    let mutated = format!(
        "{}{}",
        src[..cut].replacen("fn kb(f: &Facts)", "fn knowledge(f: &Facts)", 1),
        &src[cut..]
    );
    assert!(
        mutated != src,
        "the test is stale: the kb row's constructor"
    );
    let problems = scan(&mutated).expect_err("a renamed row constructor");
    assert!(
        problems.iter().any(|p| p.contains("`fn kb(f: &Facts)`")),
        "{problems:?}"
    );
}

#[test]
fn vocabulary_scan_fails_on_a_state_it_cannot_read() {
    let src = services_src();
    let (code, tests) = src.split_at(src.find(TESTS_START).expect("the module's tests"));
    let at = |what: &str, from: usize| {
        from + code[from..]
            .find(what)
            .unwrap_or_else(|| panic!("the test is stale: no `{what}` in services/mod.rs"))
    };
    let splice =
        |at: usize, cut: usize, with: &str| format!("{}{with}{}", &code[..at], &code[at + cut..]);
    // each case: what it does, the mutated code, a phrase only the rule that refuses it writes;
    // the scan before R1-R3 read every one of them clean
    let mut cases: Vec<(&str, String, &str)> = Vec::new();

    // R1: a facts type's method decides the gateway row's state
    let unset = at(
        "ServiceState::Unset",
        at("GatewayState::CarrierPending => Computed::new(", 0),
    );
    cases.push((
        "a facts type's method",
        splice(unset, "ServiceState::Unset".len(), "state.service_state()"),
        "the state `state.service_state()`",
    ));
    // R1: the agent's rule moved to another module
    cases.push((
        "a rule of another module",
        splice(at("child_rule(id, \"agent\"", 0), 0, "rules::"),
        "`rules::child_rule` is not a fn of this file",
    ));
    // R1: the agent row publishes the rule's state with a code of its own
    let new = "Computed::new(id, s, code, reason)";
    cases.push((
        "a code beside the rule's state",
        splice(
            at(new, 0),
            new.len(),
            "Computed::new(id, s, \"agent_down\", reason)",
        ),
        "its code is not the second name of the `let` that binds `s`",
    ));
    // R1: the model row remaps the rule's code after the call
    let semi = at(";\n", at("child_rule(id, \"model server\"", 0)) + 2;
    cases.push((
        "a code remapped after the call",
        splice(
            semi,
            0,
            "        let code = if code == \"health_pending\" {\n            \
             \"model_loading\"\n        } else {\n            code\n        };\n",
        ),
        "`code` is bound again or assigned",
    ));
    // R2: the rule hands on a state another module built
    let stopped = "ChildDisplay::Stopped => (";
    cases.push((
        "a rule handing on another module's state",
        splice(
            at(stopped, at("\nfn child_rule(", 0)),
            stopped.len(),
            "ChildDisplay::Stopped => super::probe::stopped_rule(what),\n        _ if false => (",
        ),
        "fn child_rule, whose result is a row's state, calls `super::probe::stopped_rule`",
    ));
    // R3: the kb row sets its code after the call
    cases.push((
        "a code set after the call",
        splice(
            at("out.detail = ", at("\nfn kb(f: &Facts) -> Computed {", 0)),
            0,
            "out.reason_code = \"verify_late\";\n    ",
        ),
        "assignment(s) to a `.reason_code` field",
    ));
    // R3: the meshtastic row sets a state after the call
    cases.push((
        "a state set after the call",
        splice(
            at(
                "out.actions = ",
                at("\nfn meshtastic(f: &Facts) -> Computed {", 0),
            ),
            0,
            "out.state = held_state(m);\n    ",
        ),
        "assignment(s) to a `.state` field",
    ));
    // R3: an alias of `ServiceState` (then `State::Error` in a rule is no `ServiceState::Error`)
    cases.push((
        "an alias of ServiceState",
        splice(
            at("\nfn child_rule(", 0),
            0,
            "\nuse ServiceState as State;\n",
        ),
        "an alias of `ServiceState`",
    ));
    // R3: a second struct literal sets a code `Computed::new` never saw
    cases.push((
        "a second Computed literal",
        format!(
            "{code}\nfn stray(c: Computed) -> Computed {{\n    Computed {{\n        \
             reason_code: \"verify_late\",\n        ..c\n    }}\n}}\n"
        ),
        "2 `Computed` struct literals",
    ));

    let mut missed = Vec::new();
    for (what, mutated, phrase) in &cases {
        let outcome = match scan(&format!("{mutated}{tests}")) {
            Ok(_) => "scanned clean".to_string(),
            Err(p) if p.iter().any(|x| x.contains(phrase)) => continue,
            Err(p) => p.join("; "),
        };
        missed.push(format!("{what}: {outcome}"));
    }
    assert!(
        missed.is_empty(),
        "the scan let these through:\n{}",
        missed.join("\n")
    );
}

#[test]
fn vocabulary_no_file_but_the_services_module_builds_a_row() {
    let problems = computed_built_elsewhere(&crate_sources());
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    let file = |path: &str, text: &str| (path.to_string(), text.to_string());
    let problems = computed_built_elsewhere(&[
        // a module builds a row itself, by the constructor or by a struct literal, path and all
        file(
            "src/daemon/relay/gateway.rs",
            "fn row() -> Computed {\n    Computed::new(ServiceId::Gateway, ServiceState::Error, \
             \"carrier_down\", \"\")\n}\n",
        ),
        file(
            "src/daemon/kb.rs",
            "fn row(c: Computed) -> crate::daemon::services::Computed {\n    \
             crate::daemon::services::Computed {\n        reason_code: \"verify_late\",\n        \
             ..c\n    }\n}\n",
        ),
        // a doc line, a declaration, a return type, a pattern and the module's tests build none
        file(
            "src/daemon/other.rs",
            "/// Computed once.\nstruct Computed {\n    n: u8,\n}\n\n\
             fn n(c: &Computed) -> u8 {\n    let Computed { n } = c;\n    *n\n}\n\n\
             #[cfg(test)]\nmod tests {\n    fn t() -> super::Computed {\n        \
             super::Computed { n: 1 }\n    }\n}\n",
        ),
    ]);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(
        problems[0].starts_with("src/daemon/relay/gateway.rs builds a `Computed` in 1 place"),
        "{problems:?}"
    );
    assert!(
        problems[1].starts_with("src/daemon/kb.rs builds a `Computed` in 1 place"),
        "{problems:?}"
    );
}

#[test]
fn vocabulary_scan_lets_the_other_state_fields_be_set() {
    // the file's other `state` fields (a panel's `PanelState`, the agent probe's `Option<String>`)
    // take no `ServiceState`: setting them is no side door
    let src = services_src();
    let (code, tests) = src.split_at(src.find(TESTS_START).expect("the module's tests"));
    let kb = code
        .find("\nfn kb(f: &Facts) -> Computed {")
        .expect("the kb row");
    let at = kb + code[kb..].find("out.detail = ").expect("its detail");
    for other in [
        "panel.state = PanelState::Live;\n    ",
        "probe.state = None;\n    ",
    ] {
        let mutated = format!("{}{other}{}{tests}", &code[..at], &code[at..]);
        assert!(
            scan(&mutated).is_ok(),
            "`{}` is no row state: {:?}",
            other.trim(),
            scan(&mutated).err()
        );
    }
}
