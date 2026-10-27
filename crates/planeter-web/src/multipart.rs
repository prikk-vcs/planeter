//! A bounded, streaming reader of `multipart/form-data` — exactly what the page that uploads an archive needs and no more
//! (RFC 004 part 7, T5). A browser can only send a file from a plain form as `multipart/form-data`, and the framework's reader
//! is a dependency this project does not take, so this is one hostile-input parser of its own, small enough to hold in a head.
//!
//! What it accepts, and nothing else:
//!
//! * the body starts with the delimiter (`--boundary`): **no preamble**; parts follow, each with its headers, a blank line and
//!   its body; the closing delimiter (`--boundary--`) ends it, and whatever follows is ignored;
//! * a part has at most [`HEADERS_MAX`] bytes of headers and a `Content-Disposition: form-data; name="…"` naming a field of
//!   letters, digits, `_` and `-`; **a part with a `filename` is the file**, it must be named `archive`, and **it must be the
//!   last part** (the form's text fields come first, so they can be checked before the first byte of the file is read);
//! * a text field is at most [`TEXT_MAX`] bytes of UTF-8; there are at most [`PARTS_MAX`] parts;
//! * the delimiter must be followed by `--` or CRLF: anything else is refused, not treated as data.
//!
//! The file's bytes are handed out as they arrive and are **never held**: only the last `delimiter - 1` bytes wait, because
//! they might be the start of the delimiter. The `filename` is parsed for its presence and thrown away: it never names a path.

/// The most bytes of headers one part may have.
pub const HEADERS_MAX: usize = 4096;
/// The most bytes one text field may have.
pub const TEXT_MAX: usize = 4096;
/// The most parts a body may have.
pub const PARTS_MAX: usize = 12;
/// The longest boundary (RFC 2046).
const BOUNDARY_MAX: usize = 70;

/// Why a body was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The `Content-Type` is not `multipart/form-data` with a usable boundary.
    NotMultipart,
    /// The body is not framed as described above.
    Framing,
    /// A part's headers are longer than [`HEADERS_MAX`] or malformed.
    Headers,
    /// A text field is longer than [`TEXT_MAX`].
    TextTooLong,
    /// A text field is not UTF-8.
    NotUtf8,
    /// More than [`PARTS_MAX`] parts.
    TooManyParts,
    /// A part follows the file, or the file is not named `archive`.
    FileNotLast,
    /// The body ended before the closing delimiter.
    Truncated,
}

/// What a chunk of the body turned out to contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A complete text field.
    Text { name: String, value: String },
    /// The file part begins.
    FileStart,
    /// Some bytes of the file (never empty).
    FileData(Vec<u8>),
    /// The file part is complete.
    FileEnd,
    /// The closing delimiter was seen; the rest of the body is ignored.
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// Waiting for the opening `--boundary`.
    Opening,
    /// A delimiter was read: `--` ends the body, CRLF begins a part.
    AfterDelimiter,
    Headers,
    Text(String),
    File,
    Done,
}

/// The reader. Feed it the body in chunks of any size; read the events back.
pub struct Parser {
    /// `--boundary`, the opening delimiter.
    opening: Vec<u8>,
    /// `\r\n--boundary`, the delimiter between and after parts.
    delim: Vec<u8>,
    state: State,
    buf: Vec<u8>,
    parts: usize,
    file_seen: bool,
}

/// The boundary of a `multipart/form-data` content type, or `None`.
pub fn boundary_of(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    if !parts
        .next()?
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return None;
    }
    for p in parts {
        let p = p.trim();
        let Some((k, v)) = p.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case("boundary") {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .unwrap_or(v);
            let ok = !v.is_empty()
                && v.len() <= BOUNDARY_MAX
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"'()+_,-./:=?".contains(&b))
                && !v.ends_with(' ');
            return ok.then(|| v.to_owned());
        }
    }
    None
}

impl Parser {
    /// A reader for a body of this `Content-Type`.
    pub fn new(content_type: &str) -> Result<Parser, Error> {
        let boundary = boundary_of(content_type).ok_or(Error::NotMultipart)?;
        let mut opening = b"--".to_vec();
        opening.extend_from_slice(boundary.as_bytes());
        let mut delim = b"\r\n".to_vec();
        delim.extend_from_slice(&opening);
        Ok(Parser {
            opening,
            delim,
            state: State::Opening,
            buf: Vec::new(),
            parts: 0,
            file_seen: false,
        })
    }

    /// Feed the next chunk of the body.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Event>, Error> {
        let mut out = Vec::new();
        if self.state == State::Done {
            return Ok(out);
        }
        self.buf.extend_from_slice(chunk);
        loop {
            match self.state.clone() {
                State::Done => {
                    self.buf.clear();
                    return Ok(out);
                }
                State::Opening => {
                    let n = self.opening.len();
                    if self.buf.len() < n {
                        if !self.opening.starts_with(&self.buf) {
                            return Err(Error::Framing);
                        }
                        return Ok(out);
                    }
                    if self.buf[..n] != self.opening[..] {
                        return Err(Error::Framing);
                    }
                    self.buf.drain(..n);
                    self.state = State::AfterDelimiter;
                }
                State::AfterDelimiter => {
                    if self.buf.len() < 2 {
                        return Ok(out);
                    }
                    match &self.buf[..2] {
                        b"--" => {
                            self.buf.clear();
                            self.state = State::Done;
                            out.push(Event::End);
                        }
                        b"\r\n" => {
                            self.buf.drain(..2);
                            if self.file_seen {
                                return Err(Error::FileNotLast);
                            }
                            self.parts += 1;
                            if self.parts > PARTS_MAX {
                                return Err(Error::TooManyParts);
                            }
                            self.state = State::Headers;
                        }
                        _ => return Err(Error::Framing),
                    }
                }
                State::Headers => {
                    let Some(end) = find(&self.buf, b"\r\n\r\n") else {
                        if self.buf.len() > HEADERS_MAX {
                            return Err(Error::Headers);
                        }
                        return Ok(out);
                    };
                    if end > HEADERS_MAX {
                        return Err(Error::Headers);
                    }
                    let head = std::str::from_utf8(&self.buf[..end]).map_err(|_| Error::Headers)?;
                    let (name, is_file) = disposition(head)?;
                    self.buf.drain(..end + 4);
                    if is_file {
                        if name != "archive" {
                            return Err(Error::FileNotLast);
                        }
                        self.file_seen = true;
                        self.state = State::File;
                        out.push(Event::FileStart);
                    } else {
                        self.state = State::Text(name);
                    }
                }
                State::Text(name) => {
                    let Some(i) = find(&self.buf, &self.delim) else {
                        if self.buf.len() > TEXT_MAX + self.delim.len() {
                            return Err(Error::TextTooLong);
                        }
                        return Ok(out);
                    };
                    if i > TEXT_MAX {
                        return Err(Error::TextTooLong);
                    }
                    let value =
                        String::from_utf8(self.buf[..i].to_vec()).map_err(|_| Error::NotUtf8)?;
                    self.buf.drain(..i + self.delim.len());
                    out.push(Event::Text { name, value });
                    self.state = State::AfterDelimiter;
                }
                State::File => {
                    if let Some(i) = find(&self.buf, &self.delim) {
                        if i > 0 {
                            out.push(Event::FileData(self.buf[..i].to_vec()));
                        }
                        self.buf.drain(..i + self.delim.len());
                        out.push(Event::FileEnd);
                        self.state = State::AfterDelimiter;
                    } else {
                        // The tail might be the start of the delimiter: it waits; everything before it is data.
                        let safe = self.buf.len().saturating_sub(self.delim.len() - 1);
                        if safe > 0 {
                            out.push(Event::FileData(self.buf[..safe].to_vec()));
                            self.buf.drain(..safe);
                        }
                        return Ok(out);
                    }
                }
            }
        }
    }

    /// The body is over: it must have reached its closing delimiter.
    pub fn finish(self) -> Result<(), Error> {
        if self.state == State::Done {
            Ok(())
        } else {
            Err(Error::Truncated)
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The field name and whether the part is a file, from a part's header block. Only `Content-Disposition` matters; every
/// other header (a `Content-Type` of the file, say) is allowed and ignored. The `filename` is never kept.
fn disposition(head: &str) -> Result<(String, bool), Error> {
    let mut found: Option<(String, bool)> = None;
    for line in head.split("\r\n") {
        let Some((k, v)) = line.split_once(':') else {
            return Err(Error::Headers);
        };
        if !k.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        if found.is_some() {
            return Err(Error::Headers);
        }
        let mut parts = v.split(';');
        if !parts
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("form-data")
        {
            return Err(Error::Headers);
        }
        let (mut name, mut file) = (None, false);
        for p in parts {
            let Some((pk, pv)) = p.trim().split_once('=') else {
                return Err(Error::Headers);
            };
            let pv = pv
                .trim()
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .ok_or(Error::Headers)?;
            match pk.trim().to_ascii_lowercase().as_str() {
                "name" => {
                    if name.is_some() {
                        return Err(Error::Headers);
                    }
                    name = Some(pv.to_owned());
                }
                "filename" => file = true,
                _ => {}
            }
        }
        let name = name.ok_or(Error::Headers)?;
        if name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Headers);
        }
        found = Some((name, file));
    }
    found.ok_or(Error::Headers)
}
