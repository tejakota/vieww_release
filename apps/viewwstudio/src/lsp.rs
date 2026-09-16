//! N7: speaking LSP to `rust-analyzer`.
//!
//! # Why this is a sibling of `task`, not a use of it
//!
//! `NEXT-VIEWWSTUDIO.md` states the reason and it is the whole design of this
//! module: *"`jobs.rs` streams **lines**. LSP is `Content-Length`-delimited
//! over stdio, so this needs either a framing reader beside `Task` or an
//! LSP-shaped sibling of it. Prefer the sibling, for the reason `task.rs` gives
//! for not rewriting `Job`: the line reader is correct for every *other* child
//! the studio runs, and routing the newest protocol through it would put a
//! framing bug in the build pipeline."*
//!
//! So `cargo`, `rustfmt`, `adb` and every export step keep the line reader they
//! have always had, and this is the only thing in the studio that reads a byte
//! count off a header.
//!
//! # What it does and does not do
//!
//! **This paragraph used to say "diagnostics and nothing else, for now"**, and
//! it kept saying it for two milestones after completion and hover had landed
//! three hundred lines below — so anyone reading the source to judge what the
//! studio could do read a description of a version that was long gone.
//!
//! What is here: `initialize`, `initialized`, `didOpen`, `didChange`, and back
//! from the server `publishDiagnostics`, `completion`, `hover`, `definition`
//! and `references`.
//!
//! What is **not**: rename, code actions, signature help, inlay hints,
//! semantic tokens, and workspace symbols. All of them are the same transport
//! and a message shape, and none of them are written.
//!
//! One deliberate half-measure remains, and it is stated where it happens
//! rather than here: a hover reply goes to the Output panel rather than to a
//! popup, because a popup needs a pointer-position-to-offset mapping the studio
//! does not have outside the focused field.
//!
//! # `std` only
//!
//! Like `json`, `task`, `edit_ops`, `folding`, `picker`, `scaffold` and
//! `tokens`, so the framing and the message shapes are tested under
//! `ci/standalone.sh` with no network, no GPU and no `rust-analyzer` installed.
//! **That last one is the point**: the failure this module is most likely to
//! have is a byte-counting one, and a test that needs a language server present
//! is a test that does not run on the machine where the bug is written.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

/// A diagnostic as the server reported it, in the studio's own terms.
///
/// Deliberately not `crate::state::Diagnostic`: this module is `std`-only and
/// that type lives beside the widget tree. The caller maps one to the other,
/// which is a dozen lines and keeps the standalone harness able to run this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The file, as the `file://` URI resolved back to a path.
    pub path: String,
    /// One-based, converted from LSP's zero-based line.
    pub line: u32,
    /// One-based, converted from LSP's zero-based character.
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// 1 error, 2 warning, 3 information, 4 hint — LSP's own numbering, kept
    /// rather than mapped to an enum here so a severity this module has never
    /// heard of survives to the caller instead of becoming a default.
    pub severity: u8,
    pub code: String,
    pub message: String,
}

/// What the client has to say.
#[derive(Debug)]
pub enum Event {
    /// The server answered `initialize`. Nothing may be sent before this.
    Ready,
    /// A fresh set of diagnostics for one file. **Replaces** whatever that file
    /// had — LSP's `publishDiagnostics` is the whole list for a document, and
    /// treating it as an append is how a fixed error stays on screen for ever.
    Diagnostics { path: String, reports: Vec<Report> },
    /// A reply to a completion request, with the id that asked.
    Completions { id: u32, items: Vec<Completion> },
    /// A reply to a hover request. `None` means the server had nothing to say
    /// about that position, which is different from not having replied.
    Hover { id: u32, text: Option<String> },
    /// A reply to a definition or references request: where the symbol is.
    ///
    /// One event for both, because the wire shapes are the same — `Location`
    /// or `Location[]` — and what the studio does with them differs only in
    /// whether it jumps to the first or lists them all. `Locations` is empty
    /// when the server knows the position and has nothing to point at, which
    /// is a real answer and not a failure.
    Locations { id: u32, places: Vec<Place> },
    /// The server said something this module does not model. Kept rather than
    /// dropped so an unimplemented feature is visible in the Output panel
    /// instead of looking like nothing happened.
    Other(String),
    /// The server stopped, or could not be started.
    Stopped(String),
}

/// One place in one file, as a server reports it.
///
/// Byte-free on purpose: LSP speaks in zero-based lines and UTF-16 characters,
/// and the studio's `jump_to` takes one-based line and column. The conversion
/// happens where the jump does, so this type stays exactly what came off the
/// wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The file, as a `file://` URI turned back into a path.
    pub path: std::path::PathBuf,
    /// Zero-based, as LSP sends it.
    pub line: u32,
    /// Zero-based, in UTF-16 code units, as LSP sends it.
    pub character: u32,
}

/// A frame's payload, read from a `Content-Length`-delimited stream.
///
/// # The one thing this module exists to get right
///
/// `Content-Length` counts **bytes**, not characters and not lines. A reader
/// that took a line, or that counted `char`s, works on every ASCII message a
/// server sends and then loses framing on the first diagnostic containing a
/// non-ASCII identifier or a smart quote in a message — and once framing is
/// lost it never comes back, because the next header is read from the middle of
/// a JSON body.
///
/// # Errors
///
/// Returns `None` at end of stream. A malformed header is skipped rather than
/// fatal: servers write to stderr and occasionally to stdout, and one stray
/// line should not end the session.
pub fn read_message(reader: &mut impl BufRead) -> Option<String> {
    let mut length: Option<usize> = None;
    loop {
        let mut header = String::new();
        // A header line, terminated by CRLF. `read_line` keeps the terminator,
        // which is why both are trimmed rather than one.
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // The blank line that ends the header block. Only meaningful once a
            // length has been seen; a blank line before one is noise.
            match length {
                Some(length) => {
                    let mut body = vec![0_u8; length];
                    reader.read_exact(&mut body).ok()?;
                    // Lossy rather than strict: a server that emits invalid
                    // UTF-8 has a bug, and dropping the whole session over one
                    // byte helps nobody debug it.
                    return Some(String::from_utf8_lossy(&body).into_owned());
                }
                None => continue,
            }
        }
        if let Some(value) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            length = value.trim().parse().ok();
        }
        // Every other header — `Content-Type` — is ignored on purpose. The
        // specification allows exactly one other and it has one legal value.
    }
}

/// A message, framed for the wire.
#[must_use]
pub fn frame(body: &str) -> Vec<u8> {
    // `len()` on a `str` is bytes, which is what the header must count. Written
    // out rather than left implicit because this is the line the module's whole
    // failure mode lives on.
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// A JSON string literal, escaped.
///
/// Small enough to write, and written because pulling a serialiser in for four
/// message shapes would be the largest dependency in the crate. Handles the
/// escapes the specification requires; anything below `0x20` becomes `\u00XX`.
#[must_use]
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A path as a `file://` URI.
///
/// Not a general URI encoder: the characters escaped are the ones that appear
/// in real paths and would otherwise break the JSON or the URI. A path with a
/// `#` in it is rarer than a bug in a hand-rolled percent encoder.
#[must_use]
pub fn uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::from("file://");
    for character in text.chars() {
        match character {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// The inverse of [`uri`], for the paths a server sends back.
#[must_use]
pub fn from_uri(text: &str) -> String {
    let path = text.strip_prefix("file://").unwrap_or(text);
    path.replace("%20", " ")
        .replace("%23", "#")
        .replace("%3F", "?")
}

/// The `initialize` request, for a session rooted at `root`.
#[must_use]
pub fn initialize(id: u32, root: &Path) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"initialize\",\"params\":{{\
         \"processId\":null,\"rootUri\":{},\"capabilities\":{{\"textDocument\":{{\
         \"publishDiagnostics\":{{\"relatedInformation\":false}}}}}}}}}}",
        quote(&uri(root))
    )
}

/// `initialized`, which is a notification and takes no id.
#[must_use]
pub fn initialized() -> String {
    "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}".to_owned()
}

/// `textDocument/didOpen`.
#[must_use]
pub fn did_open(path: &Path, text: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\
         \"textDocument\":{{\"uri\":{},\"languageId\":\"rust\",\"version\":1,\"text\":{}}}}}}}",
        quote(&uri(path)),
        quote(text)
    )
}

/// `textDocument/didChange`, as a whole-document replacement.
///
/// Whole-document rather than incremental, and that is a decision rather than a
/// shortcut: an incremental change list has to describe exactly the edit the
/// buffer made, and the studio's buffer reports *whole values* — see
/// `TextField`'s controlled contract. Synthesising ranges from two strings to
/// send a smaller message, when the server will apply them to reconstruct the
/// same text, is a diff computed twice and a class of desynchronisation bug for
/// nothing. Source files are kilobytes.
#[must_use]
pub fn did_change(path: &Path, version: u32, text: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{{\
         \"textDocument\":{{\"uri\":{},\"version\":{version}}},\
         \"contentChanges\":[{{\"text\":{}}}]}}}}",
        quote(&uri(path)),
        quote(text)
    )
}

/// `textDocument/completion` at a position.
///
/// # What the module docs used to say, and why this is here now
///
/// *"Hover, go-to-definition and completion are the same transport and are not
/// written."* They are the same transport, and for a **Rust** editor
/// specifically completion is not a nicety: the language is not usable at speed
/// without it, and the connection it needs was already open and already
/// carrying diagnostics.
///
/// The position is line/character, both zero-based, and `character` is in
/// **UTF-16 code units** — which is what LSP means by a column and is not what
/// anything else in this studio means. [`position_of`] does the conversion, and
/// getting it wrong is invisible on ASCII and one column out per accent on
/// everything else.
#[must_use]
pub fn completion(id: u32, path: &Path, line: u32, character: u32) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/completion\",\"params\":{{\
         \"textDocument\":{{\"uri\":{}}},\
         \"position\":{{\"line\":{line},\"character\":{character}}}}}}}",
        quote(&uri(path))
    )
}

/// `textDocument/definition` at a position.
///
/// # The most-used IDE feature the studio did not have
///
/// Goto-definition is what ⌘-click is, and a developer reading an unfamiliar
/// codebase reaches for it dozens of times an hour. The transport that carries
/// it has been open since diagnostics landed; what was missing was two message
/// shapes, which is what this and [`references`] are.
#[must_use]
pub fn definition(id: u32, path: &Path, line: u32, character: u32) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/definition\",\"params\":{{\
         \"textDocument\":{{\"uri\":{}}},\
         \"position\":{{\"line\":{line},\"character\":{character}}}}}}}",
        quote(&uri(path))
    )
}

/// `textDocument/references` at a position, including the declaration.
#[must_use]
pub fn references(id: u32, path: &Path, line: u32, character: u32) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/references\",\"params\":{{\
         \"textDocument\":{{\"uri\":{}}},\
         \"position\":{{\"line\":{line},\"character\":{character}}},\
         \"context\":{{\"includeDeclaration\":true}}}}}}",
        quote(&uri(path))
    )
}

/// `textDocument/hover` at a position.
#[must_use]
pub fn hover(id: u32, path: &Path, line: u32, character: u32) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/hover\",\"params\":{{\
         \"textDocument\":{{\"uri\":{}}},\
         \"position\":{{\"line\":{line},\"character\":{character}}}}}}}",
        quote(&uri(path))
    )
}

/// The LSP position of a byte offset in `text`.
///
/// Returns `(line, character)`, both zero-based, with `character` counted in
/// **UTF-16 code units** — the unit LSP specifies and the one nothing else in
/// this studio uses. A `char` count is right for ASCII, right for Latin-1
/// accents, and wrong for anything above the basic plane: an emoji is one
/// `char` and two UTF-16 units, so a caret after one would be reported a column
/// early and the server would complete against the wrong token.
#[must_use]
pub fn position_of(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let before = &text[..floor_boundary(text, offset)];
    let line = before.matches('\n').count();
    let column_start = before.rfind('\n').map_or(0, |at| at + 1);
    let character: usize = before[column_start..].chars().map(char::len_utf16).sum();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a file with four billion lines is not one this edits"
    )]
    {
        (line as u32, character as u32)
    }
}

/// The largest char boundary at or below `index`.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// One completion the server offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// What is shown in the list.
    pub label: String,
    /// What is inserted. Often the same as the label, and not always — a
    /// function completes as `foo(` where its label is `foo(a: u32) -> u32`.
    pub insert: String,
    /// "fn", "struct", "field" — a word for the kind, or empty.
    pub detail: String,
}

/// Pull a completion list out of a response, if that is what it is.
///
/// LSP allows two shapes for the result — a bare array, or an object with an
/// `items` array — and servers use both. Handling only one is a completion
/// list that is empty against half the servers in existence.
#[must_use]
pub fn parse_completions(body: &str) -> Option<Vec<Completion>> {
    let json = crate::json::Json::parse(body).ok()?;
    let result = json.get("result")?;
    let items = result
        .get("items")
        .and_then(crate::json::Json::as_array)
        .or_else(|| result.as_array())?;

    let mut out = Vec::new();
    for item in items {
        let Some(label) = item.str_field("label") else {
            continue;
        };
        let insert = item
            .str_field("insertText")
            .or_else(|| {
                item.get("textEdit")
                    .and_then(|edit| edit.str_field("newText"))
            })
            // The label is the fallback, and it is a *poor* one for a function
            // whose label carries its signature — so anything that looks like a
            // signature is cut at the paren rather than inserted whole.
            .map_or_else(
                || label.split('(').next().unwrap_or(label).to_owned(),
                str::to_owned,
            );
        out.push(Completion {
            label: label.to_owned(),
            insert,
            detail: item.str_field("detail").unwrap_or_default().to_owned(),
        });
    }
    Some(out)
}

/// Pull hover text out of a response.
///
/// The `contents` field has three legal shapes across LSP versions — a string,
/// an object with a `value`, or an array of either. All three are read, because
/// which one arrives depends on the server and a hover that is blank half the
/// time is a hover nobody trusts.
#[must_use]
pub fn parse_hover(body: &str) -> Option<String> {
    let json = crate::json::Json::parse(body).ok()?;
    let contents = json.get("result")?.get("contents")?;
    if let Some(text) = contents.as_str() {
        return Some(text.to_owned());
    }
    if let Some(value) = contents.str_field("value") {
        return Some(value.to_owned());
    }
    let parts = contents.as_array()?;
    let joined: Vec<String> = parts
        .iter()
        .filter_map(|part| {
            part.as_str()
                .or_else(|| part.str_field("value"))
                .map(str::to_owned)
        })
        .collect();
    (!joined.is_empty()).then(|| joined.join("\n"))
}

/// A `file://` URI back into a path.
///
/// The inverse of [`uri`], and only of [`uri`]: the three characters that were
/// escaped on the way out are unescaped on the way back, and nothing else is
/// touched. A server that percent-encodes more than that gets a path with the
/// escapes still in it, which fails to open visibly rather than opening the
/// wrong file.
#[must_use]
pub fn path_of(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = rest
        .replace("%20", " ")
        .replace("%23", "#")
        .replace("%3F", "?")
        .replace("%3f", "?");
    Some(std::path::PathBuf::from(decoded))
}

/// Pull the locations out of a `definition` or `references` reply.
///
/// # Three shapes, one answer
///
/// `textDocument/definition` may answer with a `Location`, an array of them, or
/// an array of `LocationLink`s — which name the range `targetSelectionRange`
/// instead of `range`. `references` always answers with an array. Servers
/// choose, and a client that reads one shape works against one server; this
/// reads all three, for the same reason [`parse_hover`] reads all of its.
///
/// `None` when the reply is not about locations at all, which is how the reader
/// thread tells this apart from a hover.
#[must_use]
pub fn parse_locations(body: &str) -> Option<Vec<Place>> {
    let json = crate::json::Json::parse(body).ok()?;
    let result = json.get("result")?;
    if let Some(array) = result.as_array() {
        let places: Vec<Place> = array.iter().filter_map(place_of).collect();
        // An empty array is a real answer — "no references" — but only when
        // the reply really was a location list. A hover with an empty
        // `contents` array would otherwise be read as one.
        return (!array.is_empty() && !places.is_empty()
            || array.is_empty() && has_no_contents(result))
        .then_some(places);
    }
    place_of(result).map(|place| vec![place])
}

fn has_no_contents(result: &crate::json::Json) -> bool {
    result.get("contents").is_none()
}

fn place_of(value: &crate::json::Json) -> Option<Place> {
    let uri = value
        .str_field("uri")
        .or_else(|| value.str_field("targetUri"))?;
    let range = value
        .get("range")
        .or_else(|| value.get("targetSelectionRange"))
        .or_else(|| value.get("targetRange"))?;
    let start = range.get("start")?;
    Some(Place {
        path: path_of(uri)?,
        line: start.get("line")?.as_u32()?,
        character: start.get("character")?.as_u32()?,
    })
}

/// Pull `publishDiagnostics` out of a server message, if that is what it is.
///
/// `None` for every other message, which is most of them — a server sends
/// progress, logs and responses to requests nobody is waiting on.
#[must_use]
pub fn parse_diagnostics(body: &str) -> Option<(String, Vec<Report>)> {
    let json = crate::json::Json::parse(body).ok()?;
    if json.str_field("method")? != "textDocument/publishDiagnostics" {
        return None;
    }
    let params = json.get("params")?;
    let path = from_uri(params.str_field("uri")?);
    let list = params.get("diagnostics")?.as_array()?;

    let reports = list
        .iter()
        .filter_map(|entry| {
            let range = entry.get("range")?;
            let at =
                |which: &str, field: &str| -> Option<u32> { range.path([which, field])?.as_u32() };
            Some(Report {
                path: path.clone(),
                // LSP counts lines and characters from zero and every number
                // the studio shows counts from one. Converted here, once, so
                // nothing downstream has to remember which convention it holds.
                line: at("start", "line")? + 1,
                column: at("start", "character")? + 1,
                end_line: at("end", "line")? + 1,
                end_column: at("end", "character")? + 1,
                // Absent means "the server did not say", and LSP's own guidance
                // is to treat that as the client's choice. Error, because an
                // unclassified diagnostic that hides in the warning filter is
                // worse than one that shouts.
                severity: entry
                    .get("severity")
                    .and_then(crate::json::Json::as_u32)
                    .unwrap_or(1)
                    .try_into()
                    .unwrap_or(1),
                code: entry
                    .str_field("code")
                    .map_or_else(String::new, ToOwned::to_owned),
                message: entry.str_field("message")?.to_owned(),
            })
        })
        .collect();
    Some((path, reports))
}

/// A running `rust-analyzer`, and the channel its messages arrive on.
#[derive(Debug)]
pub struct Client {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<Event>,
    next_id: Arc<AtomicU32>,
    version: u32,
}

impl Client {
    /// Start `rust-analyzer` for the project at `root`, and send `initialize`.
    ///
    /// # Errors
    ///
    /// If the binary is not on `PATH`, or its stdio could not be taken. Not
    /// having `rust-analyzer` installed is the ordinary case on a fresh
    /// machine, so this is a `Result` a caller reports rather than a panic.
    pub fn start(root: &Path, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let mut child = Command::new("rust-analyzer")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherited would put the server's log in the studio's terminal;
            // null keeps it out. Piping it would mean a third thread reading a
            // stream nothing consumes, which is how a child blocks on a full
            // pipe and looks like a hang.
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no stdout"))?;

        let (sender, events) = mpsc::channel();
        // A reader thread that will not spawn is reported, not panicked on:
        // `Client::start` already returns a `Result` the caller shows as
        // "rust-analyzer unavailable", and a studio that cannot spawn a thread
        // is a studio that should keep editing rather than exit.
        spawn_reader(stdout, sender, wake)?;

        let next_id = Arc::new(AtomicU32::new(1));
        let mut client = Self {
            child,
            stdin,
            events,
            next_id,
            version: 1,
        };
        let id = client.next_id.fetch_add(1, Ordering::Relaxed);
        client.send(&initialize(id, root))?;
        Ok(client)
    }

    /// Write one framed message.
    ///
    /// # Errors
    ///
    /// If the pipe is closed — which is what a server that has exited looks
    /// like from here.
    pub fn send(&mut self, body: &str) -> std::io::Result<()> {
        self.stdin.write_all(&frame(body))?;
        self.stdin.flush()
    }

    /// Tell the server a file is open.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn open(&mut self, path: &Path, text: &str) -> std::io::Result<()> {
        self.send(&initialized())?;
        self.send(&did_open(path, text))
    }

    /// Ask for completions at a byte offset in `text`.
    ///
    /// Returns the request id, so the reply can be matched to the request that
    /// asked for it. **Matching matters here in a way it does not for
    /// diagnostics**: a completion arrives as a reply to a specific request,
    /// and a user typing fast has several in flight at once. Showing the reply
    /// to the request from two keystroke ago is a list for a prefix they have
    /// already moved past — which looks exactly like a list that is wrong.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn request_completion(
        &mut self,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> std::io::Result<u32> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (line, character) = position_of(text, offset);
        self.send(&completion(id, path, line, character))?;
        Ok(id)
    }

    /// Ask what is under a byte offset.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn request_hover(
        &mut self,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> std::io::Result<u32> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (line, character) = position_of(text, offset);
        self.send(&hover(id, path, line, character))?;
        Ok(id)
    }

    /// Ask where the symbol under a byte offset is defined.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn request_definition(
        &mut self,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> std::io::Result<u32> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (line, character) = position_of(text, offset);
        self.send(&definition(id, path, line, character))?;
        Ok(id)
    }

    /// Ask where the symbol under a byte offset is used.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn request_references(
        &mut self,
        path: &Path,
        text: &str,
        offset: usize,
    ) -> std::io::Result<u32> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (line, character) = position_of(text, offset);
        self.send(&references(id, path, line, character))?;
        Ok(id)
    }

    /// Tell the server a file changed.
    ///
    /// # Errors
    ///
    /// If the pipe is closed.
    pub fn change(&mut self, path: &Path, text: &str) -> std::io::Result<()> {
        self.version += 1;
        let version = self.version;
        self.send(&did_change(path, version, text))
    }

    /// Everything that has arrived since the last call. Never blocks.
    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Killed rather than asked to shut down. `shutdown`/`exit` is the
        // polite sequence and takes a round trip; a studio closing does not
        // have one to wait for, and `rust-analyzer` holds no state that a
        // clean exit would flush.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read framed messages off `stdout` until it ends.
/// The `id` of a reply, if the message is one.
///
/// A reply has an `id` and no `method`; a request *from* the server has both.
/// Checking for the absence of `method` is what keeps a server-initiated
/// request — `workspace/configuration`, which rust-analyzer sends — from being
/// read as an answer to something the studio asked.
#[must_use]
fn reply_id(body: &str) -> Option<u32> {
    let json = crate::json::Json::parse(body).ok()?;
    if json.get("method").is_some() {
        return None;
    }
    json.get("id")?.as_u32()
}

fn spawn_reader(
    stdout: impl Read + Send + 'static,
    sender: Sender<Event>,
    wake: impl Fn() + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("viewwstudio-lsp".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut ready = false;
            while let Some(body) = read_message(&mut reader) {
                let event = if let Some((path, reports)) = parse_diagnostics(&body) {
                    Event::Diagnostics { path, reports }
                } else if !ready && body.contains("\"capabilities\"") {
                    ready = true;
                    Event::Ready
                } else if let Some(id) = reply_id(&body) {
                    // A reply, and which kind is decided by what parses out of
                    // it rather than by remembering what was asked — a table of
                    // outstanding requests on this thread would have to be kept
                    // in step with one on the other, and the shapes are already
                    // distinguishable.
                    if let Some(items) = parse_completions(&body) {
                        Event::Completions { id, items }
                    } else if let Some(places) = parse_locations(&body) {
                        Event::Locations { id, places }
                    } else {
                        Event::Hover {
                            id,
                            text: parse_hover(&body),
                        }
                    }
                } else {
                    // Truncated: a server's progress messages are long and the
                    // Output panel is not where somebody reads JSON.
                    Event::Other(body.chars().take(160).collect())
                };
                if sender.send(event).is_err() {
                    return;
                }
                // A message nobody asks for a frame about is a message that
                // sits in the channel until something else redraws — the same
                // trap `Reloader::wake_on_change` documents.
                wake();
            }
            let _ = sender.send(Event::Stopped("rust-analyzer exited".to_owned()));
            wake();
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(input: &str) -> Vec<String> {
        let mut reader = BufReader::new(Cursor::new(input.as_bytes().to_vec()));
        let mut out = Vec::new();
        while let Some(message) = read_message(&mut reader) {
            out.push(message);
        }
        out
    }

    #[test]
    fn two_messages_are_read_as_two() {
        let input = "Content-Length: 2\r\n\r\n{}Content-Length: 4\r\n\r\n[1,2]";
        // Deliberately wrong on the second length: `[1,2]` is five bytes and
        // the header says four, so the reader takes four and the trailing byte
        // becomes the next header's first — which is what losing framing looks
        // like, and is the caller's bug, not this function's.
        let messages = read_all(input);
        assert_eq!(messages[0], "{}");
        assert_eq!(messages[1], "[1,2");
    }

    /// **The bug this module exists to not have.** `Content-Length` counts
    /// bytes; a reader counting characters takes too few and every message
    /// after this one is read from the middle of the previous body.
    #[test]
    fn a_body_with_multi_byte_characters_is_framed_by_bytes() {
        let body = "{\"m\":\"café — naïve\"}";
        assert!(
            body.len() > body.chars().count(),
            "the fixture must be multi-byte"
        );

        let mut wire = frame(body);
        wire.extend_from_slice(&frame("{\"second\":true}"));
        let text = String::from_utf8(wire).expect("valid utf-8");

        let messages = read_all(&text);
        assert_eq!(messages.len(), 2, "framing was lost: {messages:?}");
        assert_eq!(messages[0], body);
        assert_eq!(messages[1], "{\"second\":true}");
    }

    #[test]
    fn the_header_is_written_with_the_byte_count() {
        let framed = String::from_utf8(frame("é")).expect("valid utf-8");
        assert!(framed.starts_with("Content-Length: 2\r\n\r\n"), "{framed}");
    }

    /// A server writing a stray line to stdout should cost one skipped line,
    /// not the session.
    #[test]
    fn a_line_that_is_not_a_header_is_skipped() {
        let input = format!(
            "warning: something\r\n{}",
            String::from_utf8(frame("{\"ok\":1}")).expect("valid utf-8")
        );
        assert_eq!(read_all(&input), vec!["{\"ok\":1}".to_owned()]);
    }

    #[test]
    fn a_truncated_body_ends_the_stream_rather_than_looping() {
        assert!(read_all("Content-Length: 40\r\n\r\nshort").is_empty());
    }

    #[test]
    fn quoting_escapes_what_json_requires() {
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote("a\nb"), "\"a\\nb\"");
        assert_eq!(quote("a\u{1}b"), "\"a\\u0001b\"");
        // Non-ASCII is legal unescaped, and escaping it would only make the
        // body longer — the header counts bytes either way.
        assert_eq!(quote("café"), "\"café\"");
    }

    #[test]
    fn a_path_round_trips_through_a_uri() {
        let path = Path::new("/home/someone/my code/main.rs");
        let text = uri(path);
        assert_eq!(text, "file:///home/someone/my%20code/main.rs");
        assert_eq!(from_uri(&text), "/home/someone/my code/main.rs");
    }

    #[test]
    fn diagnostics_are_parsed_and_renumbered_from_one() {
        let body = "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\
                    \"params\":{\"uri\":\"file:///p/src/main.rs\",\"diagnostics\":[\
                    {\"range\":{\"start\":{\"line\":4,\"character\":8},\
                    \"end\":{\"line\":4,\"character\":12}},\"severity\":1,\
                    \"code\":\"E0425\",\"message\":\"cannot find value\"}]}}";
        let (path, reports) = parse_diagnostics(body).expect("a diagnostics message");
        assert_eq!(path, "/p/src/main.rs");
        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!((report.line, report.column), (5, 9), "LSP counts from zero");
        assert_eq!((report.end_line, report.end_column), (5, 13));
        assert_eq!(report.severity, 1);
        assert_eq!(report.code, "E0425");
        assert_eq!(report.message, "cannot find value");
    }

    #[test]
    fn an_empty_list_is_a_message_rather_than_nothing() {
        // The message that clears a file's diagnostics once they are fixed.
        // Parsing it as `None` would leave the old ones on screen for ever.
        let body = "{\"method\":\"textDocument/publishDiagnostics\",\
                    \"params\":{\"uri\":\"file:///a.rs\",\"diagnostics\":[]}}";
        let (path, reports) = parse_diagnostics(body).expect("still a message");
        assert_eq!(path, "/a.rs");
        assert!(reports.is_empty());
    }

    #[test]
    fn every_other_message_is_not_a_diagnostics_message() {
        assert!(parse_diagnostics("{\"method\":\"window/logMessage\",\"params\":{}}").is_none());
        assert!(parse_diagnostics("{\"id\":1,\"result\":{}}").is_none());
        assert!(parse_diagnostics("not json").is_none());
    }

    #[test]
    fn the_requests_are_shaped_the_way_the_specification_asks() {
        let root = Path::new("/p");
        let hello = initialize(7, root);
        assert!(hello.contains("\"id\":7"));
        assert!(hello.contains("\"method\":\"initialize\""));
        assert!(hello.contains("\"rootUri\":\"file:///p\""));

        let open = did_open(Path::new("/p/a.rs"), "fn main() {}");
        assert!(open.contains("\"languageId\":\"rust\""));
        assert!(open.contains("\"version\":1"));

        let change = did_change(Path::new("/p/a.rs"), 4, "fn main() {}\n");
        assert!(change.contains("\"version\":4"));
        assert!(change.contains("\"contentChanges\":[{\"text\":\"fn main() {}\\n\"}]"));
        // A notification carries no id; a server that sees one answers it.
        assert!(!change.contains("\"id\""));
        assert!(!initialized().contains("\"id\""));
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    // ----- positions -----------------------------------------------------

    /// LSP counts a column in **UTF-16 code units**, which is not what
    /// anything else in this studio means by a column. Invisible on ASCII.
    #[test]
    fn a_position_is_counted_in_utf16_units() {
        assert_eq!(position_of("abc", 0), (0, 0));
        assert_eq!(position_of("abc", 3), (0, 3));
        assert_eq!(position_of("ab\ncd", 4), (1, 1));

        // `é` is two bytes and one UTF-16 unit.
        assert_eq!(position_of("é", 2), (0, 1));
        // An emoji is four bytes, one `char`, and **two** UTF-16 units — the
        // case a `chars().count()` gets wrong.
        assert_eq!(position_of("\u{1F600}", 4), (0, 2));
    }

    #[test]
    fn a_position_past_the_end_is_clamped_rather_than_panicking() {
        assert_eq!(position_of("ab", 99), (0, 2));
        assert_eq!(position_of("", 5), (0, 0));
    }

    #[test]
    fn an_offset_inside_a_character_does_not_panic() {
        // Byte 1 is the middle of `é`.
        let (line, character) = position_of("é!", 1);
        assert_eq!(line, 0);
        assert_eq!(character, 0, "floored to the boundary below");
    }

    // ----- requests ------------------------------------------------------

    #[test]
    fn a_completion_request_is_well_formed_json_rpc() {
        let body = completion(7, Path::new("/tmp/a.rs"), 3, 12);
        assert!(body.contains("\"id\":7"));
        assert!(body.contains("\"method\":\"textDocument/completion\""));
        assert!(body.contains("\"line\":3"));
        assert!(body.contains("\"character\":12"));
        assert!(crate::json::Json::parse(&body).is_ok(), "{body}");
    }

    #[test]
    fn a_hover_request_is_well_formed_json_rpc() {
        let body = hover(9, Path::new("/tmp/a.rs"), 0, 0);
        assert!(body.contains("\"method\":\"textDocument/hover\""));
        assert!(crate::json::Json::parse(&body).is_ok(), "{body}");
    }

    #[test]
    fn a_path_with_a_space_in_it_survives_the_uri() {
        let body = completion(1, Path::new("/tmp/my project/a.rs"), 0, 0);
        assert!(crate::json::Json::parse(&body).is_ok(), "{body}");
        assert!(body.contains("my%20project"), "{body}");
    }

    // ----- replies -------------------------------------------------------

    /// LSP allows two shapes for a completion result and servers use both.
    /// Handling one is a list that is empty against half the servers alive.
    #[test]
    fn both_completion_result_shapes_parse() {
        let bare = r#"{"jsonrpc":"2.0","id":2,"result":[{"label":"push"}]}"#;
        let wrapped = r#"{"jsonrpc":"2.0","id":2,"result":{"isIncomplete":false,"items":[{"label":"push"}]}}"#;
        for body in [bare, wrapped] {
            let items = parse_completions(body).expect("parsed");
            assert_eq!(items.len(), 1, "{body}");
            assert_eq!(items[0].label, "push");
        }
    }

    #[test]
    fn insert_text_wins_over_the_label() {
        let body = r#"{"id":1,"result":[{"label":"push(value: T)","insertText":"push"}]}"#;
        let items = parse_completions(body).expect("parsed");
        assert_eq!(items[0].insert, "push");
    }

    #[test]
    fn a_text_edit_is_read_when_there_is_no_insert_text() {
        let body = r#"{"id":1,"result":[{"label":"len","textEdit":{"newText":"len()"}}]}"#;
        let items = parse_completions(body).expect("parsed");
        assert_eq!(items[0].insert, "len()");
    }

    /// The label is a poor fallback for a function whose label carries its
    /// signature — inserting it whole would type `push(value: T)` into the
    /// buffer.
    #[test]
    fn a_signature_label_is_cut_at_the_paren_rather_than_inserted_whole() {
        let body = r#"{"id":1,"result":[{"label":"push(value: T)"}]}"#;
        let items = parse_completions(body).expect("parsed");
        assert_eq!(items[0].insert, "push");
    }

    #[test]
    fn an_item_with_no_label_is_skipped_rather_than_taking_the_list_down() {
        let body = r#"{"id":1,"result":[{"detail":"fn"},{"label":"ok"}]}"#;
        let items = parse_completions(body).expect("parsed");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "ok");
    }

    #[test]
    fn a_message_that_is_not_a_completion_reply_is_none() {
        assert!(parse_completions(r#"{"method":"window/logMessage"}"#).is_none());
        assert!(parse_completions("not json at all").is_none());
    }

    /// Three legal shapes for `contents` across LSP versions, and which one
    /// arrives depends on the server. A hover blank half the time is a hover
    /// nobody trusts.
    #[test]
    fn every_hover_content_shape_parses() {
        for body in [
            r#"{"id":1,"result":{"contents":"fn push"}}"#,
            r#"{"id":1,"result":{"contents":{"kind":"markdown","value":"fn push"}}}"#,
            r#"{"id":1,"result":{"contents":["fn push"]}}"#,
            r#"{"id":1,"result":{"contents":[{"value":"fn push"}]}}"#,
        ] {
            assert_eq!(parse_hover(body).as_deref(), Some("fn push"), "{body}");
        }
    }

    #[test]
    fn a_hover_with_nothing_in_it_is_none() {
        assert!(parse_hover(r#"{"id":1,"result":null}"#).is_none());
        assert!(parse_hover(r#"{"id":1}"#).is_none());
    }

    // ----- routing -------------------------------------------------------

    /// A request *from* the server has both an id and a method. Reading one as
    /// an answer to something the studio asked is how `workspace/configuration`
    /// becomes an empty completion list.
    #[test]
    fn a_server_initiated_request_is_not_mistaken_for_a_reply() {
        assert_eq!(reply_id(r#"{"id":4,"result":[]}"#), Some(4));
        assert_eq!(
            reply_id(r#"{"id":4,"method":"workspace/configuration","params":{}}"#),
            None,
            "the server asking us something is not an answer"
        );
        assert_eq!(reply_id(r#"{"method":"window/logMessage"}"#), None);
    }

    #[test]
    fn a_definition_reply_is_read_in_all_three_shapes_servers_send() {
        // A single Location.
        let one = r#"{"jsonrpc":"2.0","id":4,"result":{"uri":"file:///p/src/main.rs","range":{"start":{"line":11,"character":7},"end":{"line":11,"character":15}}}}"#;
        assert_eq!(
            parse_locations(one),
            Some(vec![Place {
                path: std::path::PathBuf::from("/p/src/main.rs"),
                line: 11,
                character: 7,
            }])
        );

        // An array of them.
        let many = r#"{"jsonrpc":"2.0","id":5,"result":[{"uri":"file:///p/a.rs","range":{"start":{"line":1,"character":2},"end":{"line":1,"character":5}}},{"uri":"file:///p/b.rs","range":{"start":{"line":9,"character":0},"end":{"line":9,"character":3}}}]}"#;
        let places = parse_locations(many).expect("two locations");
        assert_eq!(places.len(), 2);
        assert_eq!(places[1].path, std::path::PathBuf::from("/p/b.rs"));
        assert_eq!(places[1].line, 9);

        // And the LocationLink form, which names its range differently — the
        // shape a client that only reads `range` silently misses.
        let links = r#"{"jsonrpc":"2.0","id":6,"result":[{"targetUri":"file:///p/c.rs","targetRange":{"start":{"line":3,"character":0},"end":{"line":8,"character":1}},"targetSelectionRange":{"start":{"line":3,"character":7},"end":{"line":3,"character":12}}}]}"#;
        let places = parse_locations(links).expect("one link");
        assert_eq!(places[0].path, std::path::PathBuf::from("/p/c.rs"));
        assert_eq!((places[0].line, places[0].character), (3, 7));
    }

    #[test]
    fn no_references_is_an_answer_and_a_hover_is_not_one() {
        let empty = r#"{"jsonrpc":"2.0","id":7,"result":[]}"#;
        assert_eq!(parse_locations(empty), Some(Vec::new()));

        // A hover reply must not be read as a location list — they arrive on
        // the same channel and are told apart by shape, so this is the case
        // that would send the editor jumping nowhere.
        let hover = r#"{"jsonrpc":"2.0","id":8,"result":{"contents":{"kind":"markdown","value":"fn main()"}}}"#;
        assert_eq!(parse_locations(hover), None);
        assert_eq!(parse_hover(hover).as_deref(), Some("fn main()"));
    }

    #[test]
    fn a_uri_survives_the_round_trip_through_a_path_with_a_space() {
        let path = std::path::Path::new("/Users/a b/proj/src/main.rs");
        assert_eq!(path_of(&uri(path)).as_deref(), Some(path));
    }

    #[test]
    fn the_definition_and_references_requests_name_the_right_methods() {
        let path = std::path::Path::new("/p/src/main.rs");
        let go = definition(3, path, 10, 4);
        assert!(
            go.contains("\"method\":\"textDocument/definition\""),
            "{go}"
        );
        assert!(go.contains("\"line\":10"), "{go}");

        let refs = references(4, path, 10, 4);
        assert!(
            refs.contains("\"method\":\"textDocument/references\""),
            "{refs}"
        );
        assert!(
            refs.contains("\"includeDeclaration\":true"),
            "the declaration is a reference people expect in the list: {refs}"
        );
    }
}
