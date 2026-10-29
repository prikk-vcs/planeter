//! The HTTP core (RFC 004 part 9, T1): everything the program says to a server goes through here.
//!
//! * **https only.** A plain `http://` address is refused before any connection; there is no `--insecure`, and a redirect is
//!   reported as one, never followed (`max_redirects(0)`); cookies are not compiled in.
//! * **Streaming, bounded.** A download goes to `<dest>.partial` and is renamed on completion; an interrupted one resumes with
//!   `Range` and `If-Range`; a body is never held in memory except small JSON answers, which are read up to a limit. An upload is
//!   streamed from a file with an exact `Content-Length`.
//! * **The token** is added to a request here and nowhere else, and is in no error text, log or `--verbose` line.
//! * **Failures are sentences.** A TLS failure maps to one plain sentence with the next step; the raw reason (rustls' own words)
//!   is kept in [`Failure::detail`] and shown only under `--verbose`.
//! * The usual `HTTPS_PROXY` / `NO_PROXY` variables are honoured (ureq reads them), and the `User-Agent` names this program.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::catalog::{Key, Lang, size as show_size, t};
use crate::exit::{Failure, LOCAL, NETWORK, REFUSED, USAGE};
use crate::token::Token;

/// The most a small JSON answer (an advertisement, a report) may be.
pub const MAX_JSON_BYTES: u64 = 32 << 20;
/// The most of an error body that is read (and kept).
pub const MAX_ERROR_BODY: u64 = 64 << 10;

/// The times a request may take.
#[derive(Debug, Clone)]
pub struct Timeouts {
    pub connect: Duration,
    /// From the request sent to the answer's head.
    pub response: Duration,
    /// Between two reads of a body.
    pub idle: Duration,
    /// The whole call, body included.
    pub total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(10),
            response: Duration::from_secs(60),
            idle: Duration::from_secs(30),
            total: Duration::from_secs(4 * 3600),
        }
    }
}

/// The server's `{code, message, hint}` error body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub hint: String,
    /// The whole body as JSON, for the fields a command reads (`reason`, `quota`, `limit`, …).
    pub raw: serde_json::Value,
}

/// What a server answered when it did not say yes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub error: Option<ApiError>,
    pub retry_after: Option<u64>,
}

/// Why a request produced no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// The server answered, and the answer was a refusal (4xx/5xx).
    Server(Box<Answer>),
    Net(NetProblem),
}

/// A failure before an answer arrived (or one that is not the server's word).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetProblem {
    UnknownIssuer(String),
    WrongName(String),
    Expired(String),
    TlsOther(String),
    Unreachable(String),
    Timeout(String),
    /// The connection ended before the whole body arrived; a partial file is kept for the next run.
    CutShort(String),
    /// A 3xx: reported, never followed.
    Redirect(String),
    /// More than the server said (or than the caller allows).
    TooLong,
    Proxy(String),
    /// The answer is not what a planeter server says.
    BadAnswer(String),
    Other(String),
}

/// A client for one server.
pub struct Http {
    agent: ureq::Agent,
    token: Option<Token>,
    lang: Lang,
    verbose: bool,
    host: String,
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        text.push_str(" / ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    text
}

/// Map ureq's error to a problem. TLS reasons are rustls' own words, matched by what they say (the wrapping differs between
/// ureq versions; the reason strings are rustls' and are stable enough to match, and a test pins the five cases).
fn classify(e: &ureq::Error) -> NetProblem {
    let reason = chain(e);
    let lower = reason.to_ascii_lowercase();
    if lower.contains("unknownissuer")
        || lower.contains("unknown issuer")
        || lower.contains("unknownca")
    {
        return NetProblem::UnknownIssuer(reason);
    }
    if lower.contains("not valid for name") || lower.contains("notvalidforname") {
        return NetProblem::WrongName(reason);
    }
    if lower.contains("expired") || lower.contains("notvalidyet") || lower.contains("not yet valid")
    {
        return NetProblem::Expired(reason);
    }
    if lower.contains("invalid peer certificate")
        || lower.contains("invalidcertificate")
        || lower.contains("tls")
        || lower.contains("handshake")
        || lower.contains("received fatal alert")
    {
        return NetProblem::TlsOther(reason);
    }
    match e {
        ureq::Error::Timeout(_) => NetProblem::Timeout(reason),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
            NetProblem::Unreachable(reason)
        }
        ureq::Error::InvalidProxyUrl => NetProblem::Proxy(reason),
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                NetProblem::Timeout(reason)
            }
            std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe => NetProblem::CutShort(reason),
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::NotFound
            | std::io::ErrorKind::AddrNotAvailable => NetProblem::Unreachable(reason),
            _ => NetProblem::Other(reason),
        },
        ureq::Error::Protocol(_) => NetProblem::BadAnswer(reason),
        _ => NetProblem::Other(reason),
    }
}

fn classify_io(e: &std::io::Error) -> NetProblem {
    let reason = chain(e);
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            NetProblem::Timeout(reason)
        }
        // ureq reports a body that ends before its Content-Length as an UnexpectedEof, a timeout as a TimedOut; a TLS or
        // protocol fault inside a read arrives as `Other`.
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe => NetProblem::CutShort(reason),
        _ => {
            let lower = reason.to_ascii_lowercase();
            if lower.contains("timeout") || lower.contains("timed out") {
                NetProblem::Timeout(reason)
            } else if lower.contains("unexpected end")
                || lower.contains("eof")
                || lower.contains("close_notify")
            {
                NetProblem::CutShort(reason)
            } else {
                NetProblem::Other(reason)
            }
        }
    }
}

impl Http {
    /// A client. `cacert`, when given, **replaces** the built-in trust anchors with the certificates in the file.
    pub fn new(
        host: &str,
        cacert: Option<&Path>,
        token: Option<Token>,
        timeouts: &Timeouts,
        lang: Lang,
        verbose: bool,
    ) -> Result<Http, Failure> {
        // One provider, named once. Installing a second time is not an error the program can have.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut tls = TlsConfig::builder();
        if let Some(path) = cacert {
            let shown = path.display().to_string();
            let pem = std::fs::read(path).map_err(|e| {
                Failure::new(
                    LOCAL,
                    t(lang, Key::NetCacertUnreadable)
                        .with("path", &shown)
                        .text(),
                )
                .with_detail(e.to_string())
            })?;
            let certs: Vec<Certificate<'static>> = ureq::tls::parse_pem(&pem)
                .filter_map(|item| match item {
                    Ok(ureq::tls::PemItem::Certificate(c)) => Some(c),
                    _ => None,
                })
                .collect();
            if certs.is_empty() {
                return Err(Failure::new(
                    LOCAL,
                    t(lang, Key::NetCacertUnreadable)
                        .with("path", &shown)
                        .text(),
                ));
            }
            tls = tls.root_certs(RootCerts::new_with_certs(&certs));
        }
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .user_agent(format!("planeter-client/{}", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(timeouts.connect))
            .timeout_recv_response(Some(timeouts.response))
            .timeout_per_read(Some(timeouts.idle))
            .timeout_global(Some(timeouts.total))
            .tls_config(tls.build())
            .build();
        Ok(Http {
            agent: ureq::Agent::new_with_config(config),
            token,
            lang,
            verbose,
            host: host.to_owned(),
        })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    fn get(&self, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut req = self.agent.get(url);
        if let Some(tok) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", tok.expose()));
        }
        req
    }

    /// Turn a response that is not a success into the error it is.
    fn refusal(&self, mut resp: ureq::http::Response<ureq::Body>) -> HttpError {
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            return HttpError::Net(NetProblem::Redirect(location));
        }
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_ERROR_BODY)
            .read_to_vec()
            .unwrap_or_default();
        let error = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|raw| {
                let text = |k: &str| raw.get(k).and_then(|v| v.as_str()).map(str::to_owned);
                Some(ApiError {
                    code: text("code")?,
                    message: text("message").unwrap_or_default(),
                    hint: text("hint").unwrap_or_default(),
                    raw,
                })
            });
        HttpError::Server(Box::new(Answer {
            status,
            error,
            retry_after,
        }))
    }

    /// `GET` a small JSON answer, read up to `MAX_JSON_BYTES`.
    pub fn get_json(&self, url: &str) -> Result<serde_json::Value, HttpError> {
        let resp = self
            .get(url)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| HttpError::Net(classify(&e)))?;
        if resp.status().as_u16() != 200 {
            return Err(self.refusal(resp));
        }
        let mut resp = resp;
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
            .map_err(|e| HttpError::Net(classify(&e)))?;
        serde_json::from_slice(&body)
            .map_err(|e| HttpError::Net(NetProblem::BadAnswer(e.to_string())))
    }

    /// `GET` into `dest`, resumably. `if_none_match`: a validator the caller holds for a finished copy (a `304` is
    /// [`Saved::NotModified`]). `max_bytes`: the most the caller will take. `progress` is called with (bytes so far, total if
    /// known) now and then; it prints whole lines, never a redraw.
    pub fn get_file(
        &self,
        url: &str,
        dest: &Path,
        if_none_match: Option<&str>,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Saved, HttpError> {
        let partial = with_suffix(dest, ".partial");
        let sidecar = with_suffix(dest, ".partial.etag");
        let io_err = |e: std::io::Error| HttpError::Net(NetProblem::Other(e.to_string()));
        // A partial file resumes only with the validator it was started under.
        let mut have = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        let started_etag = std::fs::read_to_string(&sidecar)
            .ok()
            .filter(|s| !s.trim().is_empty());
        if have > 0 && started_etag.is_none() {
            let _ = std::fs::remove_file(&partial);
            have = 0;
        }
        for attempt in 0..2 {
            let mut req = self.get(url);
            if have > 0 {
                if let Some(etag) = &started_etag {
                    req = req
                        .header("Range", format!("bytes={have}-"))
                        .header("If-Range", etag.trim());
                }
            } else if let Some(v) = if_none_match {
                req = req.header("If-None-Match", v);
            }
            let mut resp = req.call().map_err(|e| HttpError::Net(classify(&e)))?;
            let status = resp.status().as_u16();
            match status {
                304 => return Ok(Saved::NotModified),
                416 if attempt == 0 => {
                    // The partial file is longer than the artifact now: start again.
                    let _ = std::fs::remove_file(&partial);
                    let _ = std::fs::remove_file(&sidecar);
                    have = 0;
                    continue;
                }
                200 | 206 => {}
                _ => return Err(self.refusal(resp)),
            }
            let etag = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let declared = resp.body().content_length();
            let (mut file, start) = if status == 206 && have > 0 {
                let f = OpenOptions::new()
                    .append(true)
                    .open(&partial)
                    .map_err(io_err)?;
                (f, have)
            } else {
                if let Some(e) = &etag {
                    std::fs::write(&sidecar, e).map_err(io_err)?;
                } else {
                    let _ = std::fs::remove_file(&sidecar);
                }
                (File::create(&partial).map_err(io_err)?, 0)
            };
            let total = declared.map(|d| d + start);
            if declared.is_some_and(|d| d + start > max_bytes)
                || (declared.is_none() && start == 0 && false)
            {
                let _ = std::fs::remove_file(&partial);
                let _ = std::fs::remove_file(&sidecar);
                return Err(HttpError::Net(NetProblem::TooLong));
            }
            let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
            let mut buf = vec![0u8; 64 * 1024];
            let mut got = start;
            let mut last_report = 0u64;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        got += n as u64;
                        if got > max_bytes || total.is_some_and(|t| got > t) {
                            drop(file);
                            let _ = std::fs::remove_file(&partial);
                            let _ = std::fs::remove_file(&sidecar);
                            return Err(HttpError::Net(NetProblem::TooLong));
                        }
                        file.write_all(&buf[..n]).map_err(io_err)?;
                        if got - last_report >= (total.unwrap_or(0) / 10).max(8 << 20) {
                            last_report = got;
                            progress(got, total);
                        }
                    }
                    Err(e) => {
                        let _ = file.flush();
                        return Err(HttpError::Net(classify_io(&e)));
                    }
                }
            }
            file.flush().map_err(io_err)?;
            drop(file);
            if total.is_some_and(|t| got != t) {
                // Shorter than it said: keep the partial for the next run.
                return Err(HttpError::Net(NetProblem::CutShort(format!(
                    "{got} of {} bytes",
                    total.unwrap_or(0)
                ))));
            }
            std::fs::rename(&partial, dest).map_err(io_err)?;
            let _ = std::fs::remove_file(&sidecar);
            return Ok(Saved::Saved { etag, bytes: got });
        }
        Err(HttpError::Net(NetProblem::Other(
            "the download did not restart".into(),
        )))
    }

    /// `POST` a body of `prefix` followed by the whole of `file`, with an exact `Content-Length`, streamed. Returns the status,
    /// the answer's body (bounded) and `Retry-After`.
    pub fn post_framed(
        &self,
        url: &str,
        prefix: &[u8],
        file: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<Posted, HttpError> {
        let io_err = |e: std::io::Error| HttpError::Net(NetProblem::Other(e.to_string()));
        let f = File::open(file).map_err(io_err)?;
        let len = prefix.len() as u64 + f.metadata().map_err(io_err)?.len();
        struct Counting<'a, R: Read> {
            inner: R,
            sent: u64,
            total: u64,
            next: u64,
            progress: &'a mut dyn FnMut(u64, u64),
        }
        impl<R: Read> Read for Counting<'_, R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                self.sent += n as u64;
                if self.sent >= self.next {
                    self.next = self.sent + (self.total / 10).max(8 << 20);
                    (self.progress)(self.sent, self.total);
                }
                Ok(n)
            }
        }
        let mut body = Counting {
            inner: std::io::Cursor::new(prefix.to_vec()).chain(f),
            sent: 0,
            total: len,
            next: (len / 10).max(8 << 20),
            progress,
        };
        let mut req = self
            .agent
            .post(url)
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", len.to_string())
            .header("Accept", "application/json");
        if let Some(tok) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", tok.expose()));
        }
        let resp = req
            .send(ureq::SendBody::from_reader(&mut body))
            .map_err(|e| HttpError::Net(classify(&e)))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(self.refusal(resp));
        }
        let mut resp = resp;
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
            .map_err(|e| HttpError::Net(classify(&e)))?;
        Ok(Posted {
            status,
            body: bytes,
        })
    }

    /// The sentence(s) for an error, in the person's language, with the raw cause kept as the detail.
    pub fn failure(&self, e: &HttpError) -> Failure {
        failure(self.lang, &self.host, e, self.verbose)
    }
}

/// A finished download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    Saved { etag: Option<String>, bytes: u64 },
    NotModified,
}

/// A finished upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub status: u16,
    pub body: Vec<u8>,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The Failure for an HTTP error. Server refusals that a command words itself (propose) are handled there first; this is the
/// general case: the server's own message and hint when it gave them, else a sentence for the status.
pub fn failure(lang: Lang, host: &str, e: &HttpError, verbose: bool) -> Failure {
    let _ = verbose;
    match e {
        HttpError::Server(a) => server_failure(lang, a),
        HttpError::Net(n) => net_failure(lang, host, n),
    }
}

fn server_failure(lang: Lang, a: &Answer) -> Failure {
    let wait = a.retry_after.unwrap_or(30);
    let verbatim = a
        .error
        .as_ref()
        .map(|e| format!("{} ({}): {} {}", e.code, a.status, e.message, e.hint));
    let message = match (a.status, a.error.as_ref().map(|e| e.code.as_str())) {
        (404, _) => t(lang, Key::ServerNotFound).text(),
        (401, _) => t(lang, Key::ServerSignIn).text(),
        (403, _) => t(lang, Key::ServerForbidden).text(),
        (429, _) => t(lang, Key::ServerRateLimited).with("seconds", wait).text(),
        (503, Some("busy")) => t(lang, Key::ServerBusy).with("seconds", wait).text(),
        (503, Some("transport_off")) => t(lang, Key::ServerOff).text(),
        (413, _) | (422, Some("too_large")) => t(lang, Key::ServerTooLarge).text(),
        (503, _) => t(lang, Key::ServerUnavailable).text(),
        (_, Some(_)) => {
            let e = a.error.as_ref().expect("matched Some");
            t(lang, Key::ServerRefused)
                .with("message", &e.message)
                .with("hint", &e.hint)
                .text()
        }
        (status, None) => t(lang, Key::ServerRefusedPlain)
            .with("status", status)
            .text(),
    };
    let mut f = Failure::new(REFUSED, message);
    if let Some(v) = verbatim {
        f = f.with_detail(v);
    }
    f
}

fn net_failure(lang: Lang, host: &str, n: &NetProblem) -> Failure {
    let (code, key, detail) = match n {
        NetProblem::UnknownIssuer(d) => (NETWORK, Key::TlsUnknownIssuer, Some(d.clone())),
        NetProblem::WrongName(d) => (NETWORK, Key::TlsWrongName, Some(d.clone())),
        NetProblem::Expired(d) => (NETWORK, Key::TlsExpired, Some(d.clone())),
        NetProblem::TlsOther(d) => (NETWORK, Key::TlsOther, Some(d.clone())),
        NetProblem::Unreachable(d) => (NETWORK, Key::NetUnreachable, Some(d.clone())),
        NetProblem::Timeout(d) => (NETWORK, Key::NetTimeout, Some(d.clone())),
        NetProblem::CutShort(d) => (NETWORK, Key::NetCutShort, Some(d.clone())),
        NetProblem::Redirect(l) => {
            return Failure::new(
                NETWORK,
                t(lang, Key::NetRedirect).with("location", l).text(),
            );
        }
        NetProblem::TooLong => (NETWORK, Key::NetTooLong, None),
        NetProblem::Proxy(d) => (NETWORK, Key::NetProxy, Some(d.clone())),
        NetProblem::BadAnswer(d) => (NETWORK, Key::NetBadAnswer, Some(d.clone())),
        NetProblem::Other(d) => (NETWORK, Key::NetOther, Some(d.clone())),
    };
    let mut f = Failure::new(code, t(lang, key).with("host", host).text());
    if let Some(d) = detail {
        f = f.with_detail(d);
    }
    f
}

/// The failure for an address the program will not use.
pub fn plain_http_failure(lang: Lang, url: &str) -> Failure {
    Failure::new(USAGE, t(lang, Key::UsagePlainHttp).with("url", url).text())
}

/// A size for the progress lines.
pub fn show(bytes: u64) -> String {
    show_size(bytes)
}
