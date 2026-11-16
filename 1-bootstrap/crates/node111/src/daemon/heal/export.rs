//! Layer 9 (§9 items 1, 3 and 4; §13 item 4's `PR.md` line; §14.5; §19 "Model or peer text that
//! smuggles a second patch or a trailer"): the export renderer, row D9.
//!
//! Pure: a draft's typed fields and the text a model or a peer wrote go in; `fix-<id>.mbox`,
//! `PR.md` and the phone's prefilled issue URL come out (§9 items 1 and 3). Nothing here reads a
//! clock, holds a token, opens a file or a socket, or spawns anything: the drafts routes render a
//! `heal/drafts` record through it with the human's "include the node id" tick (rows D47, D49), and
//! the changes list builds a kept change's issue URL with it (row D33).
//!
//! **The renderer rule (§9 item 4, normative).** Model and peer text never shapes the artefact's
//! structure:
//! - the subject is one line, control characters stripped, at most [`SUBJECT_MAX`] bytes;
//! - a body line starting `From ` is written `>From `, so `git mailsplit` finds one patch;
//! - a trailer-shaped line (`Token: value`, `Signed-off-by:` included) in agent or peer text is
//!   indented, so it is never a trailer; no `Signed-off-by:` is ever ours (§9 item 2);
//! - untrusted text in `PR.md` sits in a fenced block whose fence is longer than any backtick run
//!   inside it;
//! - the issue URL carries typed fields only (fingerprint, cause id, action names, release),
//!   percent-encoded: no byte of free text, so no mention, link or closing keyword.
//!
//! **The same rule against what `git am` reads as structure** (hardening; lock-amendment
//! candidates for §9 item 4). In a body, a line `git mailinfo` takes for the start of the patch
//! (`---`, `diff -`, `Index: `) or for the subject (`[PATCH]`) is indented as a trailer is, and a
//! scissors line (`-- >8 --`, which `git am --scissors` cuts at) has the two characters of each
//! mark spaced apart. What `git am` strips from a subject's start (`Re:`, blanks, `:`, `[…]`
//! groups) is stripped before the subject is written, so the commit's subject is the one the
//! person previewed; a title with nothing left, or shaped as an attribution trailer (`<token>-by:
//! …`), is not a subject: the subject falls back to `fix <id>`, and the commit never takes its
//! subject from the body. An RFC 2047 opener `=?` in the subject is written `= ?`, so no encoded
//! word decodes into a line break. Every untrusted line loses its control characters but the tab,
//! and the Unicode line, paragraph and bidi controls, so no terminal escape or bidi override hides
//! a line from the person who reads the file before sending it (§19: a peer's and a model's text is
//! data). In the diff, so the patch still applies, only the lines `git apply` matches against the
//! release's tree (a hunk's context and removals, as its `@@` header counts them) stay byte for
//! byte; every other line (additions, headers, text before, between or after the files) keeps only
//! its tab and a final CR, and, as in the body, is written `>From ` when it starts `From ` and
//! indented when it is trailer-shaped, so the only `base-commit:` a tool reads is the pinned one.
//!
//! **The author.** The node is named by its short label, the node's alias; with no alias, by
//! [`ANONYMOUS_LABEL`], never the L2 §7 operator label, which is a function of the NodeId and would
//! link as the NodeId does. The full NodeId is written only when the caller passes
//! `include_node_id` (§9 item 1): in `PR.md`'s author line and as the mbox's `Node-Id:` trailer.

use crate::calver::Calver;
use id111::encoding::NodeIdDisplay;
use id111::keyset::NodeId;
use std::fmt;
use std::fmt::Write as _;

/// The first line of every export: git's format-patch separator with its zero hash and its fixed
/// datestamp, so no wall clock is read (§9 item 1).
pub const MBOX_FIRST_LINE: &str =
    "From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001";

/// The author's address: an RFC 2606 name the human replaces with `git commit --amend
/// --reset-author` (§9 item 1).
pub const AUTHOR_EMAIL: &str = "node@111.invalid";

/// The subject's cap in bytes (§9 item 4): the title `git am` keeps as the commit's subject (the
/// mail's `[PATCH] ` is stripped by `git am`).
pub const SUBJECT_MAX: usize = 72;

/// The author node's label when the node has no alias (§9 item 1; see the module doc).
pub const ANONYMOUS_LABEL: &str = "111 node";

/// The label's cap in characters: the alias cap of the settings route (`ALIAS_MAX`).
pub const LABEL_MAX_CHARS: usize = 64;

/// Where a phone's prefilled issue opens (§9 item 3): the project's repository, the one the
/// updater's release fallback names.
pub const ISSUE_NEW_URL: &str = "https://github.com/111-Network/os/issues/new";

/// The name of the pull request text (§9 item 1).
pub const PR_FILE_NAME: &str = "PR.md";

/// At most this many tools are named (`Assisted-by:`); at least one always is.
pub const MAX_TOOLS: usize = 8;

/// At most this many action names in an issue URL.
pub const MAX_ISSUE_ACTIONS: usize = 8;

/// `PR.md`'s fixed "not verified" line for a `kb` draft (§13 item 4: "its `PR.md` says the lint did
/// not run on the node").
pub const KB_NOT_VERIFIED: &str = "the lint did not run on the node: run \
     `python3 0-knowledge/_meta/build.py --check` in your checkout before you send";

/// `PR.md`'s fixed "not verified" line for a `code` draft (§8.4, §14.5: "drafted and applies
/// cleanly, not built").
pub const CODE_NOT_VERIFIED: &str = "not built: the node checks only that the diff applies \
     cleanly to the release's source; nothing was compiled or tested";

/// The id families the export names (§18.2): the prefix plus 12 lower-case hex digits.
const DRAFT_ID_PREFIX: &str = "dft-";
const INCIDENT_ID_PREFIX: &str = "inc-";
const ID_HEX: usize = 12;

/// A tool's name and version: 1-32 bytes of ASCII letters, digits, `.`, `_`, `+`, `-`.
const TOOL_TOKEN_MAX: usize = 32;
/// A fingerprint subject: a service id, a component id (`^[a-z0-9][a-z0-9_-]{0,39}$`) or a fixed
/// daemon subject (§3.4).
const SUBJECT_ID_MAX: usize = 40;
/// A reason code and a cause id (§6.2: cause ids ≤ 24 B).
const CODE_MAX: usize = 24;
/// An action name of the closed vocabulary (§7.2).
const ACTION_NAME_MAX: usize = 48;

/// Why a draft or an issue cannot be rendered. A value is never echoed: it may be foreign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// A typed field outside its form.
    Invalid { field: &'static str },
    /// No tool named: an export always names what drafted it (§9 item 1).
    NoTools,
    /// More entries than the renderer writes.
    TooMany { field: &'static str, max: usize },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Invalid { field } => write!(f, "{field}: not of its typed form"),
            ExportError::NoTools => f.write_str("tools: an export names at least one tool"),
            ExportError::TooMany { field, max } => write!(f, "{field}: more than {max}"),
        }
    }
}

impl std::error::Error for ExportError {}

fn invalid(field: &'static str) -> ExportError {
    ExportError::Invalid { field }
}

/// A draft's class (§18.4 `Draft.class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftClass {
    /// A `patch` task's diff (§8.3, §8.4).
    Code,
    /// A `kb_draft` (§13 item 4).
    Kb,
}

impl DraftClass {
    /// The contract's name.
    pub fn as_str(self) -> &'static str {
        match self {
            DraftClass::Code => "code",
            DraftClass::Kb => "kb",
        }
    }
}

/// The author node as the export names it (§9 item 1).
#[derive(Debug, Clone, Copy)]
pub struct Author<'a> {
    /// The node's alias (`state/settings.json`): the short label. Sanitised here; `None` or a
    /// label with nothing left reads [`ANONYMOUS_LABEL`].
    pub alias: Option<&'a str>,
    /// The node's identity: written only when the caller passes `include_node_id`.
    pub node_id: &'a NodeId,
}

/// A tool an `Assisted-by:` trailer names, with its version (§9 item 1): typed tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    name: String,
    version: String,
}

impl Tool {
    /// A tool by name and version, each 1-32 bytes of ASCII letters, digits, `.`, `_`, `+` and
    /// `-`, the first a letter or a digit.
    pub fn new(name: &str, version: &str) -> Result<Tool, ExportError> {
        if !is_tool_token(name) {
            return Err(invalid("tool.name"));
        }
        if !is_tool_token(version) {
            return Err(invalid("tool.version"));
        }
        Ok(Tool {
            name: name.to_owned(),
            version: version.to_owned(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// One draft to export (§9 item 1): its typed fields, and the text a model or a peer wrote, marked
/// untrusted, which the renderer rule governs.
#[derive(Debug, Clone, Copy)]
pub struct DraftExport<'a> {
    /// The draft's id, `dft-` plus 12 hex (§18.2); the file is `fix-<id>.mbox`.
    pub id: &'a str,
    pub class: DraftClass,
    /// The incident it answers, `inc-` plus 12 hex, if any.
    pub incident_id: Option<&'a str>,
    pub author: Author<'a>,
    /// Untrusted: the title the agent wrote; the subject.
    pub title: &'a str,
    /// Untrusted: the commit message's body the agent wrote.
    pub body: &'a str,
    /// Untrusted: the incident's diagnosis as the record holds it; `PR.md` quotes it.
    pub diagnosis: Option<&'a str>,
    /// The verify log `111d` recorded; untrusted, for it carries the diff's paths.
    pub verify_log: &'a str,
    /// Untrusted: what was not verified, as the proposal listed it (a `kb_draft`'s
    /// `not_verified`); `PR.md` adds the class's fixed line first.
    pub not_verified: &'a [String],
    /// The tools that drafted it, each with its version: 1 to [`MAX_TOOLS`].
    pub tools: &'a [Tool],
    /// Untrusted: the unified diff, checked by §8.4 to apply cleanly.
    pub diff: &'a str,
    /// The release's tagged commit when the `[source]` pin names it: 40 or 64 lower-case hex.
    pub base_commit: Option<&'a str>,
}

/// The rendered export of one draft: the mbox and `PR.md` (§9 item 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// `fix-<id>.mbox`, a `git am` mbox holding exactly one patch.
    pub mbox: String,
    /// `PR.md`, the pull request text.
    pub pr_md: String,
}

/// Renders both files of a draft (§9 item 1).
pub fn render(d: &DraftExport<'_>, include_node_id: bool) -> Result<Export, ExportError> {
    Ok(Export {
        mbox: render_mbox(d, include_node_id)?,
        pr_md: render_pr_md(d, include_node_id)?,
    })
}

/// The mbox's file name, `fix-<id>.mbox` (§9 item 1).
pub fn mbox_file_name(id: &str) -> Result<String, ExportError> {
    if !is_family_id(id, DRAFT_ID_PREFIX) {
        return Err(invalid("id"));
    }
    Ok(format!("fix-{id}.mbox"))
}

/// `fix-<id>.mbox` (§9 item 1): the fixed first line, `From:`, `Subject: [PATCH] <title>`, the
/// body, the `Assisted-by:` trailers, the diff, and `base-commit:` when the `[source]` pin names
/// the tagged commit. The MIME lines declare the UTF-8 the subject and the body may hold, as
/// `git format-patch` does.
pub fn render_mbox(d: &DraftExport<'_>, include_node_id: bool) -> Result<String, ExportError> {
    check(d)?;
    let mut m = String::with_capacity(1024 + d.body.len() + d.diff.len());
    m.push_str(MBOX_FIRST_LINE);
    m.push('\n');
    let _ = writeln!(m, "From: {} <{AUTHOR_EMAIL}>", short_label(d.author.alias));
    let _ = writeln!(m, "Subject: [PATCH] {}", subject(d.title, d.id));
    m.push_str("MIME-Version: 1.0\n");
    m.push_str("Content-Type: text/plain; charset=UTF-8\n");
    m.push_str("Content-Transfer-Encoding: 8bit\n");
    m.push('\n');
    let body = clean_lines(d.body);
    for line in &body {
        m.push_str(&mbox_body_line(line));
        m.push('\n');
    }
    if !body.is_empty() {
        m.push('\n');
    }
    // The last paragraph of the message, and the only trailers in it: ours.
    for t in d.tools {
        let _ = writeln!(m, "Assisted-by: {} {}", t.name, t.version);
    }
    if include_node_id {
        let _ = writeln!(m, "Node-Id: {}", node_hex(d.author.node_id));
    }
    m.push_str("---\n");
    push_diff(&mut m, d.diff);
    if let Some(commit) = d.base_commit {
        let _ = write!(m, "\nbase-commit: {commit}\n");
    }
    Ok(m)
}

/// `PR.md` (§9 item 1): fix id, author node, incident id, the diagnosis as a quoted block, the
/// verify log, what was not verified, the tools and versions; for a `kb` draft "the lint did not
/// run on the node" (§13 item 4). Every untrusted text sits in a fence longer than any backtick
/// run inside it (§9 item 4).
pub fn render_pr_md(d: &DraftExport<'_>, include_node_id: bool) -> Result<String, ExportError> {
    check(d)?;
    let mut p = String::with_capacity(1024 + d.verify_log.len());
    let _ = writeln!(p, "## 111 fix {}", d.id);
    p.push('\n');
    let _ = writeln!(p, "- Fix id: {}", code_span(d.id));
    let _ = writeln!(p, "- Class: {}", code_span(d.class.as_str()));
    let label = code_span(&short_label(d.author.alias));
    if include_node_id {
        let id = code_span(&node_hex(d.author.node_id));
        let _ = writeln!(p, "- Author node: {label}, NodeId {id}");
    } else {
        let _ = writeln!(p, "- Author node: {label}");
    }
    match d.incident_id {
        Some(i) => {
            let _ = writeln!(p, "- Incident: {}", code_span(i));
        }
        None => p.push_str("- Incident: none\n"),
    }
    p.push('\n');
    p.push_str(
        "Drafted on a 111 node and exported for a person to review and send. Each fenced block \
         below is text the node's agent wrote or the node recorded, quoted as data.\n",
    );

    p.push_str("\n### Diagnosis\n\n");
    let diagnosis = d.diagnosis.map(clean_lines).unwrap_or_default();
    if diagnosis.is_empty() {
        p.push_str("none recorded\n");
    } else {
        push_fenced(&mut p, &diagnosis, "> ");
    }

    p.push_str("\n### Verify log\n\n");
    let log = clean_lines(d.verify_log);
    if log.is_empty() {
        p.push_str("none recorded\n");
    } else {
        push_fenced(&mut p, &log, "");
    }

    p.push_str("\n### Not verified\n\n");
    let fixed = match d.class {
        DraftClass::Code => CODE_NOT_VERIFIED,
        DraftClass::Kb => KB_NOT_VERIFIED,
    };
    let _ = writeln!(p, "- {fixed}");
    let items: Vec<String> = d.not_verified.iter().flat_map(|i| clean_lines(i)).collect();
    if !items.is_empty() {
        p.push('\n');
        push_fenced(&mut p, &items, "");
    }

    p.push_str("\n### Tools and versions\n\n");
    for t in d.tools {
        let _ = writeln!(p, "- {} {}", code_span(&t.name), code_span(&t.version));
    }
    Ok(p)
}

/// What a phone's prefilled issue is about (§9 item 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// A kept typed change (§18.4 `Change.issue_url`).
    KeptChange,
    /// A `kb` draft (§18.4 `Draft.issue_url`).
    KbDraft,
}

impl IssueKind {
    fn title(self) -> &'static str {
        match self {
            IssueKind::KeptChange => "repair kept",
            IssueKind::KbDraft => "KB draft",
        }
    }

    fn words(self) -> &'static str {
        match self {
            IssueKind::KeptChange => "a kept typed change",
            IssueKind::KbDraft => "a KB draft",
        }
    }
}

/// The typed fields a prefilled issue carries, and nothing else (§9 items 3 and 4).
#[derive(Debug, Clone, Copy)]
pub struct IssueFields<'a> {
    pub kind: IssueKind,
    /// The fingerprint (§3.4): the layer, 0 to 9.
    pub layer: u8,
    /// The fingerprint's subject: a service id, a component id or a daemon subject.
    pub subject: &'a str,
    /// The fingerprint's reason code.
    pub reason: &'a str,
    /// The cause the diagnosis named (§6.2), if any.
    pub cause_id: Option<&'a str>,
    /// The typed actions' names, from the closed vocabulary (§7.2): compiled in, never text read
    /// at run time.
    pub actions: &'a [&'static str],
    /// The running release.
    pub release: Calver,
}

/// The phone's prefilled GitHub issue URL (§9 item 3): `title` and `body` built from typed fields
/// only, percent-encoded (RFC 3986: every byte but the unreserved ones).
pub fn issue_url(f: &IssueFields<'_>) -> Result<String, ExportError> {
    if f.layer > 9 {
        return Err(invalid("layer"));
    }
    if !is_code(f.subject, SUBJECT_ID_MAX) {
        return Err(invalid("subject"));
    }
    if !is_code(f.reason, CODE_MAX) {
        return Err(invalid("reason"));
    }
    if f.cause_id.is_some_and(|c| !is_code(c, CODE_MAX)) {
        return Err(invalid("cause_id"));
    }
    if f.actions.len() > MAX_ISSUE_ACTIONS {
        return Err(ExportError::TooMany {
            field: "actions",
            max: MAX_ISSUE_ACTIONS,
        });
    }
    if f.actions.iter().any(|a| !is_action_name(a)) {
        return Err(invalid("actions"));
    }
    let fingerprint = format!("({}, {}, {})", f.layer, f.subject, f.reason);
    let title = format!("{}: {fingerprint}", f.kind.title());
    let cause = f
        .cause_id
        .map_or_else(|| "none".to_owned(), |c| format!("`{c}`"));
    let actions = if f.actions.is_empty() {
        "none".to_owned()
    } else {
        f.actions
            .iter()
            .map(|a| format!("`{a}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut body = String::new();
    body.push_str(
        "A 111 node's Repairs page wrote this issue from typed fields only: it carries no text a \
         model or a peer wrote.\n\n",
    );
    let _ = writeln!(body, "- Kind: {}", f.kind.words());
    let _ = writeln!(body, "- Fingerprint: `{fingerprint}`");
    let _ = writeln!(body, "- Cause: {cause}");
    let _ = writeln!(body, "- Actions: {actions}");
    let _ = writeln!(body, "- Release: `{}`", f.release);
    Ok(format!(
        "{ISSUE_NEW_URL}?title={}&body={}",
        percent_encode(&title),
        percent_encode(&body)
    ))
}

/// The author node's short label (§9 item 1): the alias's letters, digits, spaces, `-`, `_`, `.`
/// and `'` (nothing a mail header or Markdown reads as structure), its whitespace runs one space,
/// at most [`LABEL_MAX_CHARS`] characters; [`ANONYMOUS_LABEL`] when nothing is left.
pub fn short_label(alias: Option<&str>) -> String {
    let words: Vec<String> = alias
        .unwrap_or("")
        .split_whitespace()
        .map(|w| w.chars().filter(|&c| is_label_char(c)).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    let joined: String = words.join(" ").chars().take(LABEL_MAX_CHARS).collect();
    let label = joined.trim_end();
    if label.is_empty() {
        ANONYMOUS_LABEL.to_owned()
    } else {
        label.to_owned()
    }
}

fn is_label_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '\'')
}

fn check(d: &DraftExport<'_>) -> Result<(), ExportError> {
    if !is_family_id(d.id, DRAFT_ID_PREFIX) {
        return Err(invalid("id"));
    }
    if d.incident_id
        .is_some_and(|i| !is_family_id(i, INCIDENT_ID_PREFIX))
    {
        return Err(invalid("incident_id"));
    }
    if d.base_commit.is_some_and(|c| !is_commit(c)) {
        return Err(invalid("base_commit"));
    }
    if d.tools.is_empty() {
        return Err(ExportError::NoTools);
    }
    if d.tools.len() > MAX_TOOLS {
        return Err(ExportError::TooMany {
            field: "tools",
            max: MAX_TOOLS,
        });
    }
    Ok(())
}

fn is_family_id(s: &str, prefix: &str) -> bool {
    s.len() == prefix.len() + ID_HEX
        && s.starts_with(prefix)
        && s.as_bytes()[prefix.len()..]
            .iter()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_commit(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_tool_token(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=TOOL_TOKEN_MAX).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'+' | b'-'))
}

/// A fingerprint subject, a reason code or a cause id: lower-case ASCII letters, digits, `_` and
/// `-`, the first a letter or a digit.
fn is_code(s: &str, max: usize) -> bool {
    let b = s.as_bytes();
    (1..=max).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
}

/// An action name (`service.restart`, `rns.interface.upsert`): lower-case ASCII letters, digits,
/// `_` and `.`, the first a letter.
fn is_action_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=ACTION_NAME_MAX).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.'))
}

fn node_hex(id: &NodeId) -> String {
    NodeIdDisplay::new(id.as_bytes()).hex()
}

/// Dropped from every untrusted line: the C0 and C1 controls but the tab, and the Unicode line,
/// paragraph and bidi controls.
fn is_dropped(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(
            c,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

/// The subject (§9 item 4): one line, its whitespace runs one space, the dropped characters gone,
/// `=?` written `= ?`, what `git am` strips from a subject's start already stripped
/// ([`git_cleanup_subject`]), at most [`SUBJECT_MAX`] bytes cut at a character boundary: the
/// subject the commit gets is the one written. A title with nothing left, or shaped as an
/// attribution trailer (`Signed-off-by: …`), reads `fix <id>`; so `git am` never finds an empty
/// subject and never takes the body's first paragraph for it.
fn subject(title: &str, id: &str) -> String {
    let words: Vec<String> = title
        .split_whitespace()
        .map(|w| w.chars().filter(|&c| !is_dropped(c)).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    let joined = words.join(" ").replace("=?", "= ?");
    let cut = cut_at_boundary(git_cleanup_subject(&joined), SUBJECT_MAX).trim_end();
    if cut.is_empty() || is_attribution_trailer(cut) {
        format!("fix {id}")
    } else {
        cut.to_owned()
    }
}

/// What `git mailinfo` keeps of a subject (`mailinfo.c`'s `cleanup_subject`, as `git am` runs it
/// without `-k`): from the start, one after another, an `Re:` in any case when more follows it, a
/// blank, a `:` and a `[…]` group go; then the blanks at either end (git's `isspace`: space, tab,
/// LF, CR). What is left starts with nothing it strips, so `git am` and `git am -b` keep the
/// subject written from it whole.
fn git_cleanup_subject(s: &str) -> &str {
    let mut s = s;
    loop {
        s = match s.as_bytes() {
            [b'r' | b'R', b'e' | b'E', b':', _, ..] => &s[3..],
            [b' ' | b'\t' | b':', ..] => &s[1..],
            [b'[', ..] => match s.find(']') {
                Some(end) => &s[end + 1..],
                None => break,
            },
            _ => break,
        };
    }
    s.trim_matches([' ', '\t', '\n', '\r'])
}

fn cut_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A trailer-shaped line whose token ends `-by` (`Signed-off-by`, `Co-authored-by`, …): what a
/// tool reading line starts takes for an attribution, even on a subject line.
fn is_attribution_trailer(line: &str) -> bool {
    let token = trailer_token(line);
    token.is_some_and(|t| t.len() > 3 && t[t.len() - 3..].eq_ignore_ascii_case("-by"))
}

/// git's trailer shape (`trailer.c`'s `find_separator`, the default separator `:`): a token of
/// ASCII letters, digits and `-` from the line's start, optional blanks, then `:`. Returns the
/// token.
fn trailer_token(line: &str) -> Option<&str> {
    let b = line.as_bytes();
    let token = b
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'-')
        .count();
    if token == 0 {
        return None;
    }
    let blanks = b[token..]
        .iter()
        .take_while(|c| matches!(c, b' ' | b'\t'))
        .count();
    (b.get(token + blanks) == Some(&b':')).then(|| &line[..token])
}

/// A trailer-shaped line (§9 item 4), or git's generated `(cherry picked from commit ` prefix,
/// which git also counts as a trailer.
fn is_trailer_shaped(line: &str) -> bool {
    trailer_token(line).is_some() || line.starts_with("(cherry picked from commit ")
}

/// What `git mailinfo` takes for the start of the patch (`patchbreak`: `diff -`, `Index: `, a
/// `---` line) or, at the head of a body, for the subject (`[PATCH]`).
fn is_patch_shaped(line: &str) -> bool {
    line.starts_with("diff -")
        || line.starts_with("Index: ")
        || line.starts_with("---")
        || line.starts_with("[PATCH]")
}

/// Untrusted text as lines (§9 item 4): `\r\n`, `\r`, `\n` and the Unicode line ends each end
/// one, the dropped characters go, and the blank lines at either end go.
fn clean_lines(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n");
    let mut lines: Vec<String> = text
        .split(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'])
        .map(|l| l.chars().filter(|&c| !is_dropped(c)).collect())
        .collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let first = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(lines.len());
    lines.drain(..first);
    lines
}

/// One line of the mbox's body (§9 item 4): `From ` written `>From `; a trailer-shaped or
/// patch-shaped line indented; a scissors line's marks spaced apart.
fn mbox_body_line(line: &str) -> String {
    let out = if line.starts_with("From ") {
        format!(">{line}")
    } else if is_trailer_shaped(line) || is_patch_shaped(line) {
        format!(" {line}")
    } else {
        line.to_owned()
    };
    if is_scissors_line(out.as_bytes()) {
        space_scissors(&out)
    } else {
        out
    }
}

/// A line of untrusted text in `PR.md`: a trailer-shaped line indented (§9 item 4), so a squash
/// commit made from the pull request text never holds it as a trailer.
fn md_line(line: &str) -> String {
    if is_trailer_shaped(line) {
        format!(" {line}")
    } else {
        line.to_owned()
    }
}

/// `git mailinfo`'s `is_scissors_line`, byte for byte (its `isspace` is git's: space, tab, LF,
/// CR), over the line and the line end git reads with it: a line `git am --scissors` cuts the
/// message at.
fn is_scissors_line(line: &[u8]) -> bool {
    let n = line.len() + 1;
    let at = |k: usize| -> u8 {
        match line.get(k) {
            Some(&b) => b,
            None if k == line.len() => b'\n',
            None => 0,
        }
    };
    let (mut scissors, mut gap, mut perforation) = (0usize, 0usize, 0usize);
    let mut in_perforation = false;
    let (mut first, mut last) = (None::<usize>, 0usize);
    let mut i = 0;
    while i < n {
        let c = at(i);
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
            if in_perforation {
                perforation += 1;
                gap += 1;
            }
            i += 1;
            continue;
        }
        last = i;
        first.get_or_insert(i);
        if c == b'-' {
            in_perforation = true;
            perforation += 1;
            i += 1;
            continue;
        }
        if matches!(
            (c, at(i + 1)),
            (b'>', b'8') | (b'8', b'<') | (b'>', b'%') | (b'%', b'<')
        ) {
            in_perforation = true;
            perforation += 2;
            scissors += 2;
            i += 2;
            continue;
        }
        in_perforation = false;
        i += 1;
    }
    let visible = first.map_or(0, |f| last - f + 1);
    scissors > 0 && visible >= 8 && visible < perforation * 3 && gap * 2 < perforation
}

/// Spaces the two characters of every scissors mark (`>8`, `8<`, `>%`, `%<`) apart: the line then
/// holds no mark, and `git am --scissors` cuts nothing at it.
fn space_scissors(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut prev = None::<char>;
    for c in line.chars() {
        if prev.is_some_and(|p| matches!((p, c), ('>', '8') | ('8', '<') | ('>', '%') | ('%', '<')))
        {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// The diff, ending with a line end (§9 item 4). The lines `git apply` matches against the
/// release's tree, a hunk's context and removal lines as its `@@ -a,b +c,d @@` header counts them,
/// are that tree as §8.4 matched it: byte for byte. Every other line (an addition, a header, text
/// before, between or after the files) is the model's text ([`push_diff_text`]).
fn push_diff(m: &mut String, diff: &str) {
    // The old and the new lines the hunk being read still holds: (0, 0) outside a hunk.
    let (mut old, mut new) = (0u64, 0u64);
    for raw in diff.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let tree = if old > 0 || new > 0 {
            hunk_line(line, &mut old, &mut new)
        } else {
            if let Some(counts) = hunk_header(line) {
                (old, new) = counts;
            }
            false
        };
        if tree {
            m.push_str(line);
        } else {
            push_diff_text(m, line);
        }
        m.push('\n');
    }
}

/// A hunk header's line counts (`git apply`'s `parse_fragment_header`): `@@ -a[,b] +c[,d] @@`, an
/// omitted count reading 1. None for any other line.
fn hunk_header(line: &str) -> Option<(u64, u64)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = hunk_range(rest)?;
    let (new, rest) = hunk_range(rest.strip_prefix(" +")?)?;
    rest.starts_with(" @@").then_some((old, new))
}

/// `a[,b]` at the start of `s`: the count `b`, 1 when omitted (a count past `u64` saturates, as
/// git's `strtoul` does), and the rest of `s`.
fn hunk_range(s: &str) -> Option<(u64, &str)> {
    let digits = |t: &str| t.bytes().take_while(u8::is_ascii_digit).count();
    let start = digits(s);
    if start == 0 {
        return None;
    }
    let s = &s[start..];
    let Some(t) = s.strip_prefix(',') else {
        return Some((1, s));
    };
    let count = digits(t);
    if count == 0 {
        return None;
    }
    Some((t[..count].parse().unwrap_or(u64::MAX), &t[count..]))
}

/// One line of a hunk that still holds `old` and `new` lines, read as `git apply`'s
/// `parse_fragment` reads it: a context line (` `, or an empty line) counts against both, a removal
/// (`-`) against `old`, an addition (`+`) against `new`, a `\ No newline at end of file` against
/// neither. True for the lines matched against the tree: context and removals. A line git refuses
/// there (it refuses the whole patch) ends the hunk, and it and what follows are the model's text.
fn hunk_line(line: &str, old: &mut u64, new: &mut u64) -> bool {
    match line.as_bytes().first() {
        None | Some(b' ') if *old > 0 && *new > 0 => {
            *old -= 1;
            *new -= 1;
            true
        }
        Some(b'-') if *old > 0 => {
            *old -= 1;
            true
        }
        Some(b'+') if *new > 0 => {
            *new -= 1;
            false
        }
        Some(b'\\') => false,
        _ => {
            (*old, *new) = (0, 0);
            false
        }
    }
}

/// A diff line that is not the release's tree: the dropped characters go but a final CR, then, as
/// in the body and judged on what is left, a line starting `From ` is written `>From ` (so `git
/// mailsplit` finds one patch) and a trailer-shaped line, a `base-commit:` or
/// `prerequisite-patch-id:` of the model's among them, is indented, so the only `base-commit:` a
/// tool reads is the one the `[source]` pin names (§9 item 1).
fn push_diff_text(m: &mut String, line: &str) {
    let (text, cr) = match line.strip_suffix('\r') {
        Some(t) => (t, true),
        None => (line, false),
    };
    let clean: String = text.chars().filter(|&c| !is_dropped(c)).collect();
    if clean.starts_with("From ") {
        m.push('>');
    } else if is_trailer_shaped(&clean) {
        m.push(' ');
    }
    m.push_str(&clean);
    if cr {
        m.push('\r');
    }
}

/// `lines` inside a fence longer than any backtick run in them (§9 item 4), each line led by
/// `quote` (`"> "` for a quoted block).
fn push_fenced(p: &mut String, lines: &[String], quote: &str) {
    let lines: Vec<String> = lines.iter().map(|l| md_line(l)).collect();
    let fence = fence_for(&lines);
    let _ = writeln!(p, "{quote}{fence}text");
    for l in &lines {
        if l.is_empty() {
            p.push_str(quote.trim_end());
        } else {
            p.push_str(quote);
            p.push_str(l);
        }
        p.push('\n');
    }
    let _ = writeln!(p, "{quote}{fence}");
}

/// Backticks one more than the longest run in `lines`, three at the least (CommonMark's fence).
fn fence_for(lines: &[String]) -> String {
    let longest = lines
        .iter()
        .map(|l| longest_backtick_run(l))
        .max()
        .unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

fn longest_backtick_run(s: &str) -> usize {
    let (mut best, mut run) = (0usize, 0usize);
    for c in s.chars() {
        if c == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

/// An inline code span whose backtick fence is longer than any run inside it.
fn code_span(s: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(s) + 1);
    if s.starts_with(['`', ' ']) || s.ends_with(['`', ' ']) {
        format!("{fence} {s} {fence}")
    } else {
        format!("{fence}{s}{fence}")
    }
}

/// RFC 3986 percent-encoding: every byte but ALPHA, DIGIT, `-`, `.`, `_` and `~`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const NODE: [u8; 32] = [0x5a; 32];

    fn tools() -> Vec<Tool> {
        vec![
            Tool::new("opencode", "1.19.5").unwrap(),
            Tool::new("111-agent", "2026.11.16").unwrap(),
        ]
    }

    fn node() -> NodeId {
        NodeId::from(NODE)
    }

    const README_BEFORE: &str = "# demo\nline one\nline two\nline three\n";
    const DIFF: &str = "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n\
                        @@ -1,4 +1,4 @@\n # demo\n line one\n-line two\n+line 2\n line three\n";

    fn draft<'a>(node: &'a NodeId, tools: &'a [Tool], nv: &'a [String]) -> DraftExport<'a> {
        DraftExport {
            id: "dft-0123456789ab",
            class: DraftClass::Code,
            incident_id: Some("inc-ba9876543210"),
            author: Author {
                alias: Some("kitchen node"),
                node_id: node,
            },
            title: "Restart rnsd once its socket is back",
            body: "rnsd exits when its shared instance socket goes away.\n\nRestart it once.",
            diagnosis: Some("The transport row reads stack_down."),
            verify_log: "drafted and applies cleanly, not built",
            not_verified: nv,
            tools,
            diff: DIFF,
            base_commit: Some("0123456789abcdef0123456789abcdef01234567"),
        }
    }

    #[test]
    fn mbox_is_the_format_patch_shape() {
        let (n, t, nv) = (node(), tools(), Vec::<String>::new());
        let m = render_mbox(&draft(&n, &t, &nv), false).unwrap();
        let want = "\
From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001
From: kitchen node <node@111.invalid>
Subject: [PATCH] Restart rnsd once its socket is back
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

rnsd exits when its shared instance socket goes away.

Restart it once.

Assisted-by: opencode 1.19.5
Assisted-by: 111-agent 2026.11.16
---
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,4 +1,4 @@
 # demo
 line one
-line two
+line 2
 line three

base-commit: 0123456789abcdef0123456789abcdef01234567
";
        assert_eq!(m, want);
        // no base-commit line when the [source] pin names no commit
        let d = DraftExport {
            base_commit: None,
            ..draft(&n, &t, &nv)
        };
        let m = render_mbox(&d, false).unwrap();
        assert!(m.ends_with(" line three\n"), "{m}");
        assert!(!m.contains("base-commit"));
        assert_eq!(
            mbox_file_name("dft-0123456789ab").unwrap(),
            "fix-dft-0123456789ab.mbox"
        );
        assert_eq!(mbox_file_name("dft-0123"), Err(invalid("id")));
    }

    #[test]
    fn pr_md_names_what_the_spec_lists() {
        let (n, t) = (node(), tools());
        let nv = vec!["the restart was not tried on a live node".to_owned()];
        let p = render_pr_md(&draft(&n, &t, &nv), false).unwrap();
        let want = "\
## 111 fix dft-0123456789ab

- Fix id: `dft-0123456789ab`
- Class: `code`
- Author node: `kitchen node`
- Incident: `inc-ba9876543210`

Drafted on a 111 node and exported for a person to review and send. Each fenced block below is \
text the node's agent wrote or the node recorded, quoted as data.

### Diagnosis

> ```text
> The transport row reads stack_down.
> ```

### Verify log

```text
drafted and applies cleanly, not built
```

### Not verified

- not built: the node checks only that the diff applies cleanly to the release's source; \
nothing was compiled or tested

```text
the restart was not tried on a live node
```

### Tools and versions

- `opencode` `1.19.5`
- `111-agent` `2026.11.16`
";
        assert_eq!(p, want);
        assert!(!p.contains("the lint did not run on the node"));
        // a kb draft says the lint did not run on the node (§13 item 4); none recorded
        let d = DraftExport {
            class: DraftClass::Kb,
            incident_id: None,
            diagnosis: None,
            verify_log: " \n",
            ..draft(&n, &t, &[])
        };
        let p = render_pr_md(&d, false).unwrap();
        assert!(
            p.contains("\n- the lint did not run on the node: run "),
            "{p}"
        );
        assert!(p.contains("- Class: `kb`\n- Author node: `kitchen node`\n- Incident: none\n"));
        assert!(p.contains("### Diagnosis\n\nnone recorded\n"), "{p}");
        assert!(p.contains("### Verify log\n\nnone recorded\n"), "{p}");
        assert!(!p.contains("not built"), "{p}");
    }

    #[test]
    fn the_node_id_only_when_asked() {
        let (n, t, nv) = (node(), tools(), Vec::<String>::new());
        let hex = "5a".repeat(32);
        let label = NodeIdDisplay::new(&NODE).label();
        for alias in [Some("kitchen node"), None] {
            let d = DraftExport {
                author: Author { alias, node_id: &n },
                ..draft(&n, &t, &nv)
            };
            let e = render(&d, false).unwrap();
            for text in [&e.mbox, &e.pr_md] {
                assert!(!text.contains(&hex), "{text}");
                // nor the operator label, a function of the NodeId
                assert!(!text.contains(&label), "{text}");
            }
            let e = render(&d, true).unwrap();
            assert!(
                e.mbox.contains(&format!("\nNode-Id: {hex}\n---\n")),
                "{}",
                e.mbox
            );
            assert!(
                e.pr_md.contains(&format!(", NodeId `{hex}`\n")),
                "{}",
                e.pr_md
            );
        }
        let d = DraftExport {
            author: Author {
                alias: None,
                node_id: &n,
            },
            ..draft(&n, &t, &nv)
        };
        let m = render_mbox(&d, false).unwrap();
        assert!(m.contains("\nFrom: 111 node <node@111.invalid>\n"), "{m}");
    }

    #[test]
    fn the_label_holds_nothing_a_header_or_markdown_reads() {
        assert_eq!(short_label(Some("  kitchen   node ")), "kitchen node");
        assert_eq!(
            short_label(Some("Mallory <mallory@example.com>")),
            "Mallory malloryexample.com"
        );
        assert_eq!(short_label(Some("\"x\" (y), @z; `w`: =?q?=")), "x y z w q");
        assert_eq!(short_label(Some("a\nFrom: b\u{202e}c\u{1b}")), "a From bc");
        assert_eq!(short_label(Some("Nœud d'été")), "Nœud d'été");
        assert_eq!(short_label(Some("<>@\u{7}")), ANONYMOUS_LABEL);
        assert_eq!(short_label(Some("")), ANONYMOUS_LABEL);
        assert_eq!(short_label(None), ANONYMOUS_LABEL);
        let long = short_label(Some(&"é".repeat(100)));
        assert_eq!(long.chars().count(), LABEL_MAX_CHARS);
        let spaced = short_label(Some(&format!("{} b", "a".repeat(63))));
        assert_eq!(spaced, "a".repeat(63), "no trailing space after the cut");
    }

    #[test]
    fn the_subject_is_one_line_of_at_most_72_bytes() {
        let id = "dft-0123456789ab";
        assert_eq!(
            subject("Fix\nFrom: x\r\nSubject: y", id),
            "Fix From: x Subject: y"
        );
        assert_eq!(
            subject("\u{1b}[31mred\u{1b}[0m\ttab\u{0}", id),
            "[31mred[0m tab"
        );
        assert_eq!(subject("a\u{202e}b\u{2028}c\u{85}d", id), "ab c d");
        assert_eq!(subject("=?UTF-8?q?x=0Ay?=", id), "= ?UTF-8?q?x=0Ay?=");
        let long = subject(&"x".repeat(300), id);
        assert_eq!(long.len(), SUBJECT_MAX);
        // a multi-byte character is never cut
        let euro = subject(&"€".repeat(40), id);
        assert_eq!(euro, "€".repeat(24));
        assert!(euro.len() <= SUBJECT_MAX);
        let cut = subject(&format!("{} €", "a".repeat(70)), id);
        assert_eq!(
            cut,
            "a".repeat(70),
            "the cut drops the partial character and the space"
        );
        // nothing left, or an attribution trailer: the typed fallback
        assert_eq!(subject(" \n\u{7} ", id), "fix dft-0123456789ab");
        assert_eq!(
            subject("Signed-off-by: M <m@example.com>", id),
            "fix dft-0123456789ab"
        );
        assert_eq!(subject("co-authored-BY : M", id), "fix dft-0123456789ab");
        // a subsystem prefix is a subject, not an attribution
        assert_eq!(
            subject("transport: restart rnsd", id),
            "transport: restart rnsd"
        );
        assert_eq!(subject("9-evolution: fix", id), "9-evolution: fix");
    }

    #[test]
    fn body_lines_never_shape_the_mail() {
        let cases = [
            ("From mallory with love", ">From mallory with love"),
            (
                "From: Mallory <m@example.com>",
                " From: Mallory <m@example.com>",
            ),
            (
                "Signed-off-by: M <m@example.com>",
                " Signed-off-by: M <m@example.com>",
            ),
            ("signed-off-by: m", " signed-off-by: m"),
            ("Reviewed-by : M", " Reviewed-by : M"),
            ("Co-authored-by:M", " Co-authored-by:M"),
            ("Index: README.md", " Index: README.md"),
            (
                "(cherry picked from commit 0123)",
                " (cherry picked from commit 0123)",
            ),
            ("---", " ---"),
            ("--- a/README.md", " --- a/README.md"),
            ("diff --git a/x b/x", " diff --git a/x b/x"),
            ("[PATCH] another subject", " [PATCH] another subject"),
            ("-- >8 --", "-- > 8 --"),
            ("--8<----", "--8 <----"),
            // unchanged: no shape git reads
            (">From already quoted", ">From already quoted"),
            (" Signed-off-by: indented", " Signed-off-by: indented"),
            ("not a trailer: two words", "not a trailer: two words"),
            ("-- a comment", "-- a comment"),
            ("x >8 y", "x >8 y"),
            ("", ""),
        ];
        for (input, want) in cases {
            let got = mbox_body_line(input);
            assert_eq!(got, want, "{input:?}");
            assert!(!is_scissors_line(got.as_bytes()), "{got:?}");
        }
        // "--- >8 ---" is a scissors line both before and after its indent
        let got = mbox_body_line("--- >8 ---");
        assert_eq!(got, " --- > 8 ---");
        assert!(is_scissors_line(b"  --- >8 ---"));
    }

    #[test]
    fn the_scissors_port_agrees_with_git() {
        // mailinfo.c's own comment and t5100's cases
        for yes in [
            "-- >8 --",
            "-- 8< --",
            "--8<--8<--8<--",
            "- - - - >8 - - - -",
            "-->8-- cut here --8<--",
            "-- %< --",
            "------------>8------------",
        ] {
            assert!(is_scissors_line(yes.as_bytes()), "{yes:?}");
            assert!(!is_scissors_line(space_scissors(yes).as_bytes()), "{yes:?}");
        }
        for no in [
            "",
            "-- 8 --",
            "a >8 b",
            "-->8",
            "--- cut here ---",
            ">8 and some words here",
        ] {
            assert!(!is_scissors_line(no.as_bytes()), "{no:?}");
        }
        assert_eq!(space_scissors(">8<%<>%"), "> 8 <% <> %");
    }

    #[test]
    fn untrusted_lines_lose_controls_and_edge_blanks() {
        assert_eq!(
            clean_lines("\n \na\r\nb\rc\u{2028}d\u{85}e\u{1b}[0m\tf\u{202e}\n\n"),
            ["a", "b", "c", "d", "e[0m\tf"]
        );
        assert!(clean_lines(" \r\n\t\n").is_empty());
    }

    #[test]
    fn the_diff_keeps_the_tree_and_drops_controls_elsewhere() {
        let diff = "diff --git a/R b/R\n--- a/R\n+++ b/R\n@@ -1,2 +1,2 @@\n ctx\u{c}\r\n\
                    -old\u{1b}\r\n+new\u{1b}[8m\r\u{202e}x\r\nFrom 0000 Mon Sep 17 00:00:00 2001\n\
                    \\ No newline at end of file";
        let mut m = String::new();
        push_diff(&mut m, diff);
        assert_eq!(
            m,
            "diff --git a/R b/R\n--- a/R\n+++ b/R\n@@ -1,2 +1,2 @@\n ctx\u{c}\r\n\
             -old\u{1b}\r\n+new[8mx\r\n>From 0000 Mon Sep 17 00:00:00 2001\n\
             \\ No newline at end of file\n"
        );
    }

    /// `lines`, each ended by a line end: a diff, or the text one renders to.
    fn joined(lines: &[&str]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    fn diff_of(lines: &[&str]) -> String {
        let mut m = String::new();
        push_diff(&mut m, &joined(lines));
        m
    }

    /// Only the lines a hunk's header counts as context or removals keep their bytes: text before
    /// the first file, a header and text after a hunk that start with ` ` or `-` lose theirs.
    #[test]
    fn only_a_hunks_tree_lines_keep_their_bytes() {
        let got = diff_of(&[
            " \u{1b}[31mred before the first file",
            "-\u{1b}[2Jan erased screen",
            "diff --git a/R b/R",
            "--- a/R\u{202e}",
            "+++ b/R",
            "@@ -1,3 +1,3 @@",
            " ctx\u{1b}",
            "",
            "-old\u{202e}",
            "+new\u{1b}",
            "@@ -9 +9 @@",
            "-nine\u{1b}",
            "+9",
            "\\ No newline at end of file\u{1b}",
            " after the hunks\u{1b}",
            "-after\u{202e}",
        ]);
        let want = joined(&[
            " [31mred before the first file",
            "-[2Jan erased screen",
            "diff --git a/R b/R",
            "--- a/R",
            "+++ b/R",
            "@@ -1,3 +1,3 @@",
            " ctx\u{1b}",
            "",
            "-old\u{202e}",
            "+new",
            "@@ -9 +9 @@",
            "-nine\u{1b}",
            "+9",
            "\\ No newline at end of file",
            " after the hunks",
            "-after",
        ]);
        assert_eq!(got, want);
        // a line git refuses inside a hunk ends it (git refuses the patch): what follows is text
        let got = diff_of(&[
            "@@ -1,3 +1,3 @@",
            " a\u{1b}",
            "x\u{1b}",
            " b\u{1b}",
            "-c\u{1b}",
            "@@ -1 +1,2 @@",
            "-d\u{1b}",
            " e\u{1b}",
            "@@ -1,2 +1 @@",
            "+f\u{1b}",
            "+g\u{1b}",
            "-h\u{1b}",
            "@@ -1 +1 @@",
            "\r",
            " i\u{1b}",
        ]);
        let want = joined(&[
            "@@ -1,3 +1,3 @@",
            " a\u{1b}",
            "x",
            " b",
            "-c",
            "@@ -1 +1,2 @@",
            "-d\u{1b}",
            " e",
            "@@ -1,2 +1 @@",
            "+f",
            "+g",
            "-h",
            "@@ -1 +1 @@",
            "\r",
            " i",
        ]);
        assert_eq!(got, want);
    }

    /// The lines of `text` starting with `prefix` (lower case), in any case.
    fn lines_starting<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
        text.lines()
            .filter(|l| l.to_ascii_lowercase().starts_with(prefix))
            .collect()
    }

    #[test]
    fn hunk_headers_are_read_as_git_reads_them() {
        assert_eq!(hunk_header("@@ -1,4 +1,5 @@"), Some((4, 5)));
        assert_eq!(hunk_header("@@ -1 +1 @@ fn main() {"), Some((1, 1)));
        assert_eq!(hunk_header("@@ -0,0 +1,2 @@\r"), Some((0, 2)));
        assert_eq!(
            hunk_header("@@ -1,99999999999999999999999 +1 @@"),
            Some((u64::MAX, 1))
        );
        for no in [
            "",
            "@@ -1,4 +1,5",
            "@@ -a +1 @@",
            "@@ -1, +1 @@",
            "@@ 1 +1 @@",
            "@@ -1 -1 @@",
            " @@ -1 +1 @@",
            "@@ -1 +1@@",
        ] {
            assert_eq!(hunk_header(no), None, "{no:?}");
        }
    }

    /// A `base-commit:` or `prerequisite-patch-id:` of the model's never stands at a line's start,
    /// nor does a `From ` that a control character hid; the pinned `base-commit:` is the one line.
    #[test]
    fn the_diff_carries_no_base_commit_or_from_line_of_its_own() {
        let (n, t, nv) = (node(), tools(), Vec::<String>::new());
        let (one, two, three) = ("1".repeat(40), "2".repeat(40), "3".repeat(40));
        let diff = format!(
            "base-commit: {one}\n\u{7}From 0000000000000000000000000000000000000000 Mon Sep 17 \
             00:00:00 2001\n{DIFF}Base-Commit: {two}\nprerequisite-patch-id: {three}\n\
             \u{200e}base-commit: {two}\n"
        );
        let pinned = "0123456789abcdef0123456789abcdef01234567";
        for base_commit in [None, Some(pinned)] {
            let d = DraftExport {
                diff: &diff,
                base_commit,
                ..draft(&n, &t, &nv)
            };
            let m = render_mbox(&d, false).unwrap();
            let bases = lines_starting(&m, "base-commit:");
            match base_commit {
                None => assert!(bases.is_empty(), "{m}"),
                Some(c) => {
                    assert_eq!(bases, [format!("base-commit: {c}")], "{m}");
                    assert!(m.ends_with(&format!("\n\nbase-commit: {c}\n")), "{m}");
                }
            }
            assert!(
                lines_starting(&m, "prerequisite-patch-id:").is_empty(),
                "{m}"
            );
            assert_eq!(lines_starting(&m, "from "), [MBOX_FIRST_LINE], "{m}");
            for want in [
                format!(" base-commit: {one}"),
                ">From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001".into(),
                format!(" Base-Commit: {two}"),
                format!(" prerequisite-patch-id: {three}"),
                format!(" base-commit: {two}"),
            ] {
                assert!(m.lines().any(|l| l == want), "{want:?} in\n{m}");
            }
        }
    }

    /// The subject is what `git am` keeps of it: git's subject cleanup (`Re:`, blanks, `:`,
    /// `[…]` groups at the start) has nothing left to strip, and never strips it to nothing.
    #[test]
    fn the_subject_is_what_git_am_keeps() {
        let (n, t, nv) = (node(), tools(), Vec::<String>::new());
        let fallback = "fix dft-0123456789ab";
        let long = format!("[{}] {}", "p".repeat(100), "x".repeat(100));
        let x72 = "x".repeat(SUBJECT_MAX);
        let cases = [
            // git leaves nothing: the typed fallback, never the body's first paragraph
            ("[x]", fallback),
            ("[WIP] : [v2]", fallback),
            ("Re: [skip ci] :", fallback),
            ("re:RE:[a][b]::", fallback),
            ("[PATCH 1/2]", fallback),
            // what git strips is not written, and what is left is judged
            ("[x] Signed-off-by: M <m@example.com>", fallback),
            ("Re: Co-authored-by: M", fallback),
            ("[skip ci] fix rnsd", "fix rnsd"),
            ("Re: Re: [x] : transport: restart", "transport: restart"),
            (": :fix", "fix"),
            (long.as_str(), x72.as_str()),
            // what git keeps stays
            ("Re:", "Re:"),
            ("Re :x", "Re :x"),
            ("Rex: y", "Rex: y"),
            ("[x", "[x"),
            ("fix [x] later", "fix [x] later"),
            ("x [a]", "x [a]"),
        ];
        for (title, want) in cases {
            assert_eq!(subject(title, "dft-0123456789ab"), want, "{title:?}");
        }
        if Command::new("git").arg("--version").output().is_err() {
            println!("skipped: git not found; git mailinfo's subject check did not run");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::write(home.join("gitconfig"), "").unwrap();
        let file = home.join("fix-dft-0123456789ab.mbox");
        for (title, want) in cases {
            let d = DraftExport {
                title,
                body: "",
                ..draft(&n, &t, &nv)
            };
            std::fs::write(&file, render_mbox(&d, false).unwrap()).unwrap();
            // as `git am` runs it, and as `git am -b` (non-PATCH brackets kept) does
            for args in [&[][..], &["-b"][..]] {
                let mut c = git(home);
                c.arg("mailinfo")
                    .args(args)
                    .arg(home.join("msg"))
                    .arg(home.join("patch"));
                let info = run(c, Some(&file));
                let kept = info.lines().find_map(|l| l.strip_prefix("Subject: "));
                assert_eq!(kept, Some(want), "{title:?} {args:?}: {info}");
            }
        }
    }

    #[test]
    fn fences_are_longer_than_any_run_inside() {
        let mut p = String::new();
        push_fenced(&mut p, &["a ```` b".to_owned(), "`".to_owned()], "");
        assert_eq!(p, "`````text\na ```` b\n`\n`````\n");
        let mut p = String::new();
        push_fenced(
            &mut p,
            &["x".to_owned(), String::new(), "Acked-by: y".to_owned()],
            "> ",
        );
        assert_eq!(p, "> ```text\n> x\n>\n>  Acked-by: y\n> ```\n");
        assert_eq!(code_span("dft-0123456789ab"), "`dft-0123456789ab`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`a"), "`` `a ``");
    }

    #[test]
    fn typed_fields_are_checked() {
        let (n, t, nv) = (node(), tools(), Vec::<String>::new());
        let base = draft(&n, &t, &nv);
        let (upper, sha256) = ("A".repeat(40), "ab".repeat(32));
        let bad = [
            (
                DraftExport {
                    id: "dft-0123456789AB",
                    ..base
                },
                invalid("id"),
            ),
            (
                DraftExport {
                    id: "inc-0123456789ab",
                    ..base
                },
                invalid("id"),
            ),
            (
                DraftExport {
                    id: "dft-0123456789abc",
                    ..base
                },
                invalid("id"),
            ),
            (
                DraftExport {
                    incident_id: Some("inc-01234567\n9ab"),
                    ..base
                },
                invalid("incident_id"),
            ),
            (
                DraftExport {
                    base_commit: Some("main"),
                    ..base
                },
                invalid("base_commit"),
            ),
            (
                DraftExport {
                    base_commit: Some(&upper),
                    ..base
                },
                invalid("base_commit"),
            ),
            (DraftExport { tools: &[], ..base }, ExportError::NoTools),
        ];
        for (d, want) in bad {
            assert_eq!(render(&d, false), Err(want.clone()));
            assert_eq!(render_pr_md(&d, false), Err(want));
        }
        let many: Vec<Tool> = (0..=MAX_TOOLS)
            .map(|_| Tool::new("t", "1").unwrap())
            .collect();
        let d = DraftExport {
            tools: &many,
            ..base
        };
        assert_eq!(
            render(&d, false),
            Err(ExportError::TooMany {
                field: "tools",
                max: MAX_TOOLS
            })
        );
        assert!(render(
            &DraftExport {
                base_commit: Some(&sha256),
                ..base
            },
            false
        )
        .is_ok());
        assert_eq!(Tool::new("open code", "1"), Err(invalid("tool.name")));
        assert_eq!(Tool::new("-x", "1"), Err(invalid("tool.name")));
        assert_eq!(Tool::new("x", ""), Err(invalid("tool.version")));
        assert_eq!(
            Tool::new("x", &"1".repeat(32)).map(|t| t.version().len()),
            Ok(32)
        );
        assert_eq!(
            Tool::new("x", &"1".repeat(33)),
            Err(invalid("tool.version"))
        );
        assert_eq!(Tool::new("x", "1\n"), Err(invalid("tool.version")));
        let e = ExportError::TooMany {
            field: "tools",
            max: 8,
        };
        assert_eq!(e.to_string(), "tools: more than 8");
        assert_eq!(invalid("id").to_string(), "id: not of its typed form");
    }

    fn release() -> Calver {
        "2026.11.16".parse().unwrap()
    }

    fn issue() -> IssueFields<'static> {
        IssueFields {
            kind: IssueKind::KeptChange,
            layer: 6,
            subject: "transport",
            reason: "stack_down",
            cause_id: Some("rnsd_socket_gone"),
            actions: &["service.restart", "rns.interface.upsert"],
            release: release(),
        }
    }

    fn percent_decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    /// The query's two values, decoded; every byte of them unreserved or `%HH`.
    fn issue_query(url: &str) -> (String, String) {
        let rest = url
            .strip_prefix(&format!("{ISSUE_NEW_URL}?title="))
            .unwrap();
        let (title, body) = rest.split_once("&body=").unwrap();
        for v in [title, body] {
            let b = v.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%' {
                    assert!(b[i + 1..i + 3]
                        .iter()
                        .all(|c| matches!(c, b'0'..=b'9' | b'A'..=b'F')));
                    i += 3;
                } else {
                    assert!(
                        b[i].is_ascii_alphanumeric() || b"-._~".contains(&b[i]),
                        "{v}"
                    );
                    i += 1;
                }
            }
        }
        (percent_decode(title), percent_decode(body))
    }

    #[test]
    fn the_issue_url_holds_typed_fields_only() {
        let url = issue_url(&issue()).unwrap();
        let (title, body) = issue_query(&url);
        assert_eq!(title, "repair kept: (6, transport, stack_down)");
        assert_eq!(
            body,
            "A 111 node's Repairs page wrote this issue from typed fields only: it carries no text \
             a model or a peer wrote.\n\n- Kind: a kept typed change\n\
             - Fingerprint: `(6, transport, stack_down)`\n- Cause: `rnsd_socket_gone`\n\
             - Actions: `service.restart`, `rns.interface.upsert`\n- Release: `2026.11.16`\n"
        );
        assert!(url.starts_with(
            "https://github.com/111-Network/os/issues/new?title=repair%20kept%3A%20%286%2C%20"
        ));
        let kb = IssueFields {
            kind: IssueKind::KbDraft,
            cause_id: None,
            actions: &[],
            ..issue()
        };
        let (title, body) = issue_query(&issue_url(&kb).unwrap());
        assert_eq!(title, "KB draft: (6, transport, stack_down)");
        assert!(body.contains("- Kind: a KB draft\n"), "{body}");
        assert!(body.contains("- Cause: none\n- Actions: none\n"), "{body}");
        // a field outside its form is refused, never written
        let (long_reason, subject40) = ("r".repeat(25), "c".repeat(40));
        let bad = [
            (
                IssueFields {
                    layer: 10,
                    ..issue()
                },
                invalid("layer"),
            ),
            (
                IssueFields {
                    subject: "Transport",
                    ..issue()
                },
                invalid("subject"),
            ),
            (
                IssueFields {
                    subject: "@mallory",
                    ..issue()
                },
                invalid("subject"),
            ),
            (
                IssueFields {
                    reason: "stack down",
                    ..issue()
                },
                invalid("reason"),
            ),
            (
                IssueFields {
                    reason: "",
                    ..issue()
                },
                invalid("reason"),
            ),
            (
                IssueFields {
                    reason: &long_reason,
                    ..issue()
                },
                invalid("reason"),
            ),
            (
                IssueFields {
                    cause_id: Some("fixes #1"),
                    ..issue()
                },
                invalid("cause_id"),
            ),
            (
                IssueFields {
                    actions: &["see https://x"],
                    ..issue()
                },
                invalid("actions"),
            ),
            (
                IssueFields {
                    actions: &["advise"; MAX_ISSUE_ACTIONS + 1],
                    ..issue()
                },
                ExportError::TooMany {
                    field: "actions",
                    max: MAX_ISSUE_ACTIONS,
                },
            ),
        ];
        for (f, want) in bad {
            assert_eq!(issue_url(&f), Err(want));
        }
        let ok = IssueFields {
            subject: &subject40,
            ..issue()
        };
        assert!(issue_url(&ok).is_ok(), "a component id of 40 bytes");
    }

    #[test]
    fn rendering_is_deterministic() {
        let (n, t, nv) = (node(), tools(), vec!["x".to_owned()]);
        let d = draft(&n, &t, &nv);
        assert_eq!(render(&d, false), render(&d, false));
        assert_eq!(render(&d, true), render(&d, true));
        assert_eq!(issue_url(&issue()), issue_url(&issue()));
    }

    /// §9: no clock, no token, no network call, no file and no process in the renderer.
    #[test]
    fn the_renderer_reads_no_clock_and_opens_nothing() {
        let src = include_str!("export.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for name in [
            "std::time",
            "SystemTime",
            "Instant",
            "std::net",
            "std::fs",
            "std::env",
            "std::process",
            "tokio",
            "ureq",
            "getrandom",
        ] {
            assert!(!code.contains(name), "{name} in the renderer");
        }
    }

    // --- G24 (4)'s renderer half: the smuggling fixtures ---------------------------------------

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        case: String,
        smuggles: String,
        title: String,
        body: String,
        diagnosis: String,
        verify_log: String,
        not_verified: Vec<String>,
        diff: String,
        subject: String,
        readme_after: String,
        #[serde(default)]
        mbox_lines: Vec<String>,
        #[serde(default)]
        pr_lines: Vec<String>,
    }

    fn fixtures() -> Vec<Fixture> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/l9-D9");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        let all: Vec<Fixture> = paths
            .iter()
            .map(|p| {
                let f: Fixture = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
                assert_eq!(p.file_stem().unwrap().to_str(), Some(f.case.as_str()));
                assert!(!f.smuggles.is_empty());
                assert!(
                    [&f.title, &f.body, &f.diagnosis]
                        .iter()
                        .any(|t| t.contains("allory")),
                    "{}: every case's free text names mallory",
                    f.case
                );
                f
            })
            .collect();
        assert!(all.len() >= 5, "the smuggling fixtures: {}", dir.display());
        all
    }

    fn fixture_draft<'a>(f: &'a Fixture, n: &'a NodeId, t: &'a [Tool]) -> DraftExport<'a> {
        DraftExport {
            id: "dft-0123456789ab",
            class: DraftClass::Code,
            incident_id: Some("inc-ba9876543210"),
            author: Author {
                alias: Some("kitchen node"),
                node_id: n,
            },
            title: &f.title,
            body: &f.body,
            diagnosis: Some(&f.diagnosis),
            verify_log: &f.verify_log,
            not_verified: &f.not_verified,
            tools: t,
            diff: &f.diff,
            base_commit: Some("0123456789abcdef0123456789abcdef01234567"),
        }
    }

    /// The commit message `git am` keeps: the lines between the headers' blank line and `---`.
    fn message_lines(mbox: &str) -> Vec<&str> {
        let after = mbox.split_once("\n\n").unwrap().1;
        after.lines().take_while(|l| *l != "---").collect()
    }

    /// Each fenced block of `PR.md`: its fence is longer than any backtick run inside, and it
    /// closes.
    fn assert_fences(pr: &str, case: &str) -> usize {
        let mut open: Option<usize> = None;
        let mut blocks = 0;
        for raw in pr.lines() {
            let line = raw
                .strip_prefix("> ")
                .or_else(|| raw.strip_prefix('>'))
                .unwrap_or(raw);
            match open {
                None if line.starts_with("```") => {
                    let n = longest_backtick_run(line);
                    assert_eq!(line, format!("{}text", "`".repeat(n)), "{case}: {line}");
                    open = Some(n);
                }
                None => {}
                Some(n) if line == "`".repeat(n) => {
                    open = None;
                    blocks += 1;
                }
                Some(n) => assert!(longest_backtick_run(line) < n, "{case}: {line} in {n}"),
            }
        }
        assert!(open.is_none(), "{case}: a fence left open");
        blocks
    }

    #[test]
    fn smuggling_fixtures_render_one_patch_and_no_trailer_of_theirs() {
        let (n, t) = (node(), tools());
        let hex = "5a".repeat(32);
        let ours = [
            "Assisted-by: opencode 1.19.5",
            "Assisted-by: 111-agent 2026.11.16",
        ];
        let url = issue_url(&issue()).unwrap();
        let mut at_cap = 0;
        for f in fixtures() {
            let case = f.case.as_str();
            let d = fixture_draft(&f, &n, &t);
            let e = render(&d, false).unwrap();
            let lines: Vec<&str> = e.mbox.lines().collect();
            // one `From ` separator line: the fixed first line
            assert_eq!(lines[0], MBOX_FIRST_LINE, "{case}");
            let froms = lines.iter().filter(|l| l.starts_with("From ")).count();
            assert_eq!(froms, 1, "{case}:\n{}", e.mbox);
            // the subject: one line, its title at most 72 B, as the fixture pins it
            let s = lines[2].strip_prefix("Subject: [PATCH] ").unwrap();
            assert!(
                s.len() <= SUBJECT_MAX && !s.chars().any(is_dropped),
                "{case}: {s:?}"
            );
            assert_eq!(s, f.subject, "{case}");
            assert_eq!(lines[1], "From: kitchen node <node@111.invalid>", "{case}");
            // no line of theirs is a trailer, a patch start or a scissors line in the message
            for l in message_lines(&e.mbox) {
                if is_trailer_shaped(l) {
                    assert!(ours.contains(&l), "{case}: a trailer of theirs: {l:?}");
                }
                assert!(!is_patch_shaped(l), "{case}: a patch start: {l:?}");
                assert!(
                    !is_scissors_line(l.as_bytes()),
                    "{case}: a scissors line: {l:?}"
                );
            }
            // no Signed-off-by: at a line's start, in either file
            for text in [&e.mbox, &e.pr_md] {
                for l in text.lines() {
                    assert!(
                        !l.to_ascii_lowercase().starts_with("signed-off-by"),
                        "{case}: {l:?}"
                    );
                }
            }
            // no control character but the tab (and a line's final CR) in either file: the
            // fixtures' tree, README_BEFORE, holds none for a hunk's lines to keep
            for text in [&e.mbox, &e.pr_md] {
                for l in text.split('\n') {
                    let l = l.strip_suffix('\r').unwrap_or(l);
                    assert!(!l.chars().any(is_dropped), "{case}: {l:?}");
                }
            }
            // one `base-commit:` line, the pinned one, last; no `prerequisite-patch-id:`
            let bases = lines_starting(&e.mbox, "base-commit:");
            assert_eq!(
                bases,
                ["base-commit: 0123456789abcdef0123456789abcdef01234567"],
                "{case}"
            );
            assert_eq!(lines.last(), bases.first(), "{case}: the pinned one last");
            assert!(
                lines_starting(&e.mbox, "prerequisite-patch-id:").is_empty(),
                "{case}"
            );
            // the fixture's own expectations: the indented trailer, `>From `, the fence lines
            for want in &f.mbox_lines {
                assert!(
                    lines.contains(&want.as_str()),
                    "{case}: {want:?} in\n{}",
                    e.mbox
                );
            }
            let pr: Vec<&str> = e.pr_md.lines().collect();
            for want in &f.pr_lines {
                assert!(
                    pr.contains(&want.as_str()),
                    "{case}: {want:?} in\n{}",
                    e.pr_md
                );
            }
            // the longer fence: three blocks (diagnosis, verify log, not verified), each closed
            assert_eq!(assert_fences(&e.pr_md, case), 3, "{case}");
            // the NodeId only when asked
            assert!(!e.mbox.contains(&hex) && !e.pr_md.contains(&hex), "{case}");
            let asked = render(&d, true).unwrap();
            assert!(
                asked.mbox.contains(&hex) && asked.pr_md.contains(&hex),
                "{case}"
            );
            // the issue URL holds no byte of free text: the same URL whatever the text, and no
            // fixture word in it
            let (title, body) = issue_query(&url);
            for text in [&title, &body, &url] {
                assert!(
                    !text.to_ascii_lowercase().contains("allory"),
                    "{case}: {text}"
                );
            }
            assert_eq!(issue_url(&issue()).unwrap(), url);
            for text in [&f.title, &f.body, &f.diagnosis] {
                let as_cause = IssueFields {
                    cause_id: Some(text),
                    ..issue()
                };
                assert_eq!(issue_url(&as_cause), Err(invalid("cause_id")), "{case}");
                let as_subject = IssueFields {
                    subject: text,
                    ..issue()
                };
                assert_eq!(issue_url(&as_subject), Err(invalid("subject")), "{case}");
            }
            if s.len() == SUBJECT_MAX {
                at_cap += 1;
            }
            git_checks(case, &f, &e.mbox);
        }
        assert!(at_cap >= 1, "a fixture's subject reaches the 72 B cap");
    }

    /// Runs `git` with no system or user configuration and a fixed committer.
    fn git(home: &Path) -> Command {
        let mut c = Command::new("git");
        for v in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_AUTHOR_DATE",
        ] {
            c.env_remove(v);
        }
        c.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("LC_ALL", "C")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.invalid");
        c
    }

    fn run(mut c: Command, input: Option<&Path>) -> String {
        if let Some(p) = input {
            c.stdin(std::fs::File::open(p).unwrap());
        }
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{c:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// `git mailsplit` finds one patch; `git mailinfo`, with and without `--scissors`, keeps our
    /// author and subject, our trailers only and our diff only; `git am` applies exactly the
    /// diff. Skipped with a printed line where `git` is absent (§9 item 4, G24 (4)).
    fn git_checks(case: &str, f: &Fixture, mbox: &str) {
        if Command::new("git").arg("--version").output().is_err() {
            println!(
                "skipped: git not found; {case}'s git mailsplit and git am checks did not run"
            );
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::write(home.join("gitconfig"), "").unwrap();
        let file = home.join("fix-dft-0123456789ab.mbox");
        std::fs::write(&file, mbox).unwrap();

        let split = home.join("split");
        std::fs::create_dir(&split).unwrap();
        let mut c = git(home);
        c.arg("mailsplit")
            .arg(format!("-o{}", split.display()))
            .arg(&file);
        assert_eq!(run(c, None).trim(), "1", "{case}: git mailsplit");
        let mails: Vec<_> = std::fs::read_dir(&split).unwrap().collect();
        assert_eq!(mails.len(), 1, "{case}: one patch");
        let mail = split.join("0001");

        for scissors in ["--no-scissors", "--scissors"] {
            let (msg, patch) = (home.join("msg"), home.join("patch"));
            let mut c = git(home);
            c.args(["mailinfo", scissors]).arg(&msg).arg(&patch);
            let info = run(c, Some(&mail));
            assert_eq!(
                info,
                format!(
                    "Author: kitchen node\nEmail: node@111.invalid\nSubject: {}\n\n",
                    f.subject
                ),
                "{case} {scissors}"
            );
            // the trailers of the commit message `git am` builds: the subject, a blank line,
            // the message
            let kept = std::fs::read_to_string(&msg).unwrap();
            let commit_msg = home.join("commit-msg");
            std::fs::write(&commit_msg, format!("{}\n\n{kept}", f.subject)).unwrap();
            let mut c = git(home);
            c.args(["interpret-trailers", "--parse"]);
            assert_eq!(
                run(c, Some(&commit_msg)),
                "Assisted-by: opencode 1.19.5\nAssisted-by: 111-agent 2026.11.16\n",
                "{case} {scissors}: the trailers"
            );
            // the whole message as rendered: no in-body header taken, no scissors cut
            let rendered: String = message_lines(mbox)
                .iter()
                .map(|l| format!("{l}\n"))
                .collect();
            assert_eq!(kept, rendered, "{case} {scissors}: the message");
            // the patch file holds our `---` line onwards, as rendered (`git mailsplit` drops
            // a CR before a line end)
            let patch = std::fs::read_to_string(&patch).unwrap();
            let ours = &mbox[mbox.find("\n---\n").unwrap() + 1..];
            assert_eq!(
                patch,
                ours.replace("\r\n", "\n"),
                "{case} {scissors}: the patch"
            );
            assert_eq!(patch.matches("diff --git ").count(), 1, "{case}: {patch}");
        }

        let repo = home.join("repo");
        let mut c = git(home);
        c.args(["init", "-q", "-b", "main"]).arg(&repo);
        run(c, None);
        std::fs::write(repo.join("README.md"), README_BEFORE).unwrap();
        let mut c = git(home);
        c.arg("-C").arg(&repo).args(["add", "README.md"]);
        run(c, None);
        let mut c = git(home);
        c.arg("-C")
            .arg(&repo)
            .env("GIT_AUTHOR_NAME", "base")
            .env("GIT_AUTHOR_EMAIL", "base@example.invalid")
            .args(["commit", "-q", "-m", "base"]);
        run(c, None);
        let mut c = git(home);
        c.arg("-C").arg(&repo).args(["am", "-q"]).arg(&file);
        run(c, None);
        let mut c = git(home);
        c.arg("-C").arg(&repo).args([
            "log",
            "-1",
            "--format=%an%n%ae%n%s%n%(trailers:only,unfold)",
        ]);
        assert_eq!(
            run(c, None),
            format!(
                "kitchen node\nnode@111.invalid\n{}\nAssisted-by: opencode 1.19.5\n\
                 Assisted-by: 111-agent 2026.11.16\n\n",
                f.subject
            ),
            "{case}: the commit"
        );
        let mut c = git(home);
        c.arg("-C")
            .arg(&repo)
            .args(["diff", "--name-only", "HEAD~1", "HEAD"]);
        assert_eq!(run(c, None), "README.md\n", "{case}: one file changed");
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            f.readme_after,
            "{case}"
        );
        assert!(!repo.join("EVIL.md").exists(), "{case}");
    }
}
