//! T1: the HTTP core, against a real https server on a loopback port (tests/common). Resume byte-identical, `304`, a redirect
//! refused, the token never printed, the TLS failure sentences for the five cases, bounded reads of hostile answers.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Reply, Req, Server, ca, full, scratch};
use planeter_client::catalog::{Key, Lang, t};
use planeter_client::exit::{NETWORK, REFUSED};
use planeter_client::http::{Http, HttpError, NetProblem, Saved, Timeouts, failure};
use planeter_client::token::Token;
use planeter_client::url::{RepoUrl, UrlError};

fn body(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn http(server: &Server, cacert: bool, token: Option<&str>, timeouts: &Timeouts) -> Http {
    let ca_path = ca();
    Http::new(
        "127.0.0.1",
        cacert.then_some(ca_path.as_path()),
        token.and_then(Token::new),
        timeouts,
        Lang::En,
        false,
    )
    .inspect(|_| {
        let _ = server;
    })
    .unwrap()
}

fn quick() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(5),
        response: Duration::from_secs(10),
        idle: Duration::from_secs(5),
        total: Duration::from_secs(30),
    }
}

fn nothing(_: u64, _: Option<u64>) {}

#[test]
fn a_download_streams_to_a_partial_file_and_is_renamed_whole() {
    let data = body(300_000);
    let served = data.clone();
    let server = Server::start(
        "good",
        Arc::new(move |_: &Req| {
            full(
                200,
                &[
                    ("ETag", "\"v1\""),
                    ("Content-Type", "application/octet-stream"),
                ],
                &served,
            )
        }),
    );
    let dir = scratch("download");
    let dest = dir.join("a.bin");
    let h = http(&server, true, None, &quick());
    let got = h
        .get_file(&server.url("/a"), &dest, None, 1 << 30, &mut nothing)
        .unwrap();
    assert_eq!(
        got,
        Saved::Saved {
            etag: Some("\"v1\"".into()),
            bytes: 300_000
        }
    );
    assert_eq!(std::fs::read(&dest).unwrap(), data);
    assert!(!dir.join("a.bin.partial").exists());
    assert!(!dir.join("a.bin.partial.etag").exists());
}

#[test]
fn an_interrupted_download_resumes_with_range_and_if_range_and_is_byte_identical() {
    let data = body(500_000);
    let served = data.clone();
    let cut = 200_000usize;
    let server = Server::start(
        "good",
        Arc::new(move |r: &Req| {
            let served = served.clone();
            match (r.header("range"), r.header("if-range")) {
                (Some(range), Some("\"v1\"")) => {
                    let from: usize = range
                        .trim_start_matches("bytes=")
                        .trim_end_matches('-')
                        .parse()
                        .unwrap();
                    let rest = served[from..].to_vec();
                    Reply::Raw(Box::new(move |w| {
                        let head = format!(
                            "HTTP/1.1 206 X\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
                            rest.len(),
                            from,
                            served.len() - 1,
                            served.len()
                        );
                        let _ = w.write_all(head.as_bytes());
                        let _ = w.write_all(&rest);
                    }))
                }
                _ => Reply::Raw(Box::new(move |w| {
                    // The whole length is promised; only `cut` bytes arrive, then the connection ends.
                    let head = format!(
                        "HTTP/1.1 200 X\r\nContent-Length: {}\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n",
                        served.len()
                    );
                    let _ = w.write_all(head.as_bytes());
                    let _ = w.write_all(&served[..cut]);
                })),
            }
        }),
    );
    let dir = scratch("resume");
    let dest = dir.join("a.bin");
    let h = http(&server, true, None, &quick());
    let first = h
        .get_file(&server.url("/a"), &dest, None, 1 << 30, &mut nothing)
        .unwrap_err();
    assert!(
        matches!(first, HttpError::Net(NetProblem::CutShort(_))),
        "{first:?}"
    );
    assert_eq!(
        std::fs::metadata(dir.join("a.bin.partial")).unwrap().len(),
        cut as u64
    );
    assert!(
        !dest.exists(),
        "a half-written file is never the destination"
    );
    let second = h
        .get_file(&server.url("/a"), &dest, None, 1 << 30, &mut nothing)
        .unwrap();
    assert_eq!(
        second,
        Saved::Saved {
            etag: Some("\"v1\"".into()),
            bytes: 500_000
        }
    );
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        data,
        "byte for byte what the server holds"
    );
    let seen = server.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].header("range"), Some("bytes=200000-"));
    assert_eq!(seen[1].header("if-range"), Some("\"v1\""));
}

#[test]
fn a_matching_validator_is_a_304_and_nothing_is_written() {
    let server = Server::start(
        "good",
        Arc::new(|r: &Req| {
            if r.header("if-none-match") == Some("\"v1\"") {
                full(304, &[("ETag", "\"v1\"")], b"")
            } else {
                full(200, &[("ETag", "\"v1\"")], b"data")
            }
        }),
    );
    let dir = scratch("304");
    let dest = dir.join("a.bin");
    let h = http(&server, true, None, &quick());
    assert_eq!(
        h.get_file(
            &server.url("/a"),
            &dest,
            Some("\"v1\""),
            1 << 30,
            &mut nothing
        )
        .unwrap(),
        Saved::NotModified
    );
    assert!(!dest.exists());
    assert!(matches!(
        h.get_file(
            &server.url("/a"),
            &dest,
            Some("\"other\""),
            1 << 30,
            &mut nothing
        )
        .unwrap(),
        Saved::Saved { .. }
    ));
}

#[test]
fn a_redirect_is_reported_and_never_followed() {
    let server = Server::start(
        "good",
        Arc::new(|_: &Req| full(302, &[("Location", "https://elsewhere.example/x")], b"")),
    );
    let dir = scratch("redirect");
    let h = http(&server, true, Some("secret-token-1"), &quick());
    for e in [
        h.get_file(
            &server.url("/a"),
            &dir.join("a"),
            None,
            1 << 30,
            &mut nothing,
        )
        .unwrap_err(),
        h.get_json(&server.url("/b")).unwrap_err(),
    ] {
        assert_eq!(
            e,
            HttpError::Net(NetProblem::Redirect("https://elsewhere.example/x".into()))
        );
        let f = h.failure(&e);
        assert_eq!(f.code, NETWORK);
        assert!(f.message.contains("never follows"), "{}", f.message);
    }
    // Two requests were made and no third: nothing was followed (and the token went only to the address that was asked).
    assert_eq!(server.seen().len(), 2);
}

#[test]
fn the_token_goes_in_the_header_and_in_no_text_the_program_can_print() {
    let token = "tok-5ecret-abc123";
    let server = Server::start(
        "good",
        Arc::new(|r: &Req| {
            if r.path == "/err" {
                full(
                    403,
                    &[("Content-Type", "application/json")],
                    br#"{"code":"forbidden","message":"No.","hint":"Ask."}"#,
                )
            } else {
                full(200, &[], br#"{"ok":true}"#)
            }
        }),
    );
    let h = http(&server, true, Some(token), &quick());
    h.get_json(&server.url("/ok")).unwrap();
    let e = h.get_json(&server.url("/err")).unwrap_err();
    let f = h.failure(&e);
    let everything = format!("{e:?} {f:?} {} {:?}", f.message, f.detail);
    assert!(!everything.contains(token), "{everything}");
    assert!(!format!("{:?}", Token::new(token).unwrap()).contains(token));
    let seen = server.seen();
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(
        seen[0].header("user-agent"),
        Some(format!("planeter-client/{}", env!("CARGO_PKG_VERSION")).as_str())
    );
    assert!(seen[0].header("cookie").is_none());
    // A failure to connect names neither the token nor the header.
    let down = Http::new(
        "127.0.0.1",
        None,
        Token::new(token),
        &quick(),
        Lang::En,
        true,
    )
    .unwrap();
    let gone = down.get_json("https://127.0.0.1:1/x").unwrap_err();
    let f = down.failure(&gone);
    assert!(!format!("{gone:?} {f:?}").contains(token));
}

#[test]
fn the_servers_error_body_is_parsed_and_its_words_are_shown() {
    let server = Server::start(
        "good",
        Arc::new(|_: &Req| {
            full(
                422,
                &[("Content-Type", "application/json")],
                br#"{"code":"unknown_ref","message":"The repository has no ref with that name.","hint":"Take the ref names from the advertisement."}"#,
            )
        }),
    );
    let h = http(&server, true, None, &quick());
    let e = h.get_json(&server.url("/x")).unwrap_err();
    let HttpError::Server(a) = &e else {
        panic!("{e:?}")
    };
    assert_eq!(a.status, 422);
    assert_eq!(a.error.as_ref().unwrap().code, "unknown_ref");
    let f = h.failure(&e);
    assert_eq!(f.code, REFUSED);
    assert!(
        f.message
            .contains("The repository has no ref with that name.")
            && f.message.contains("Take the ref names"),
        "{}",
        f.message
    );
}

/// The five TLS cases and the plain-http refusal, each one plain sentence with the next step; the raw reason only as the detail.
#[test]
fn certificate_failures_are_one_plain_sentence_each_and_the_raw_reason_is_only_the_detail() {
    let dir = scratch("tls");
    let quick = quick();
    let check = |cert: &str, cacert: bool, want: Key, raw_has: &str| {
        let server = Server::start(cert, Arc::new(|_: &Req| full(200, &[], b"{}")));
        let h = http(&server, cacert, None, &quick);
        let e = h.get_json(&server.url("/x")).unwrap_err();
        let f = h.failure(&e);
        assert_eq!(f.code, NETWORK, "{cert}: {e:?}");
        assert_eq!(
            f.message,
            t(Lang::En, want).with("host", "127.0.0.1").text(),
            "{cert}: {e:?}"
        );
        let detail = f.detail.clone().unwrap_or_default();
        assert!(
            detail.contains(raw_has),
            "{cert}: the detail {detail:?} should hold {raw_has:?}"
        );
        for word in ["UnknownIssuer", "rustls", "InvalidCertificate"] {
            assert!(
                !f.message.contains(word),
                "{cert}: the raw reason leaked into the sentence: {}",
                f.message
            );
        }
        f.message
    };
    // The authority is not known (no --cacert, the built-in list does not have the test authority).
    let unknown = check("good", false, Key::TlsUnknownIssuer, "UnknownIssuer");
    assert!(unknown.contains("--cacert"), "{unknown}");
    // A certificate from another private authority, though --cacert names this one.
    check("other", true, Key::TlsUnknownIssuer, "UnknownIssuer");
    // Another name.
    check("wrong", true, Key::TlsWrongName, "not valid for name");
    // Expired.
    check("exp", true, Key::TlsExpired, "expired");
    // Nothing is listening.
    let h = http(
        &Server::start("good", Arc::new(|_: &Req| full(200, &[], b"{}"))),
        true,
        None,
        &quick,
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let e = h
        .get_json(&format!("https://127.0.0.1:{port}/x"))
        .unwrap_err();
    assert!(
        matches!(e, HttpError::Net(NetProblem::Unreachable(_))),
        "{e:?}"
    );
    let f = failure(Lang::En, "127.0.0.1", &e, false);
    assert_eq!(
        f.message,
        t(Lang::En, Key::NetUnreachable)
            .with("host", "127.0.0.1")
            .text()
    );
    // A plain http address is refused before any connection, by the address parser.
    assert_eq!(
        RepoUrl::parse("http://127.0.0.1/a/b"),
        Err(UrlError::PlainHttp)
    );
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
}

#[test]
fn a_cacert_replaces_the_trust_anchors_and_an_unreadable_one_is_a_sentence() {
    let server = Server::start("good", Arc::new(|_: &Req| full(200, &[], b"{}")));
    let dir = scratch("cacert");
    // A file that holds no certificate.
    let junk = dir.join("junk.pem");
    std::fs::write(&junk, "not a certificate").unwrap();
    let e = Http::new("127.0.0.1", Some(&junk), None, &quick(), Lang::En, false)
        .err()
        .unwrap();
    assert_eq!(
        e.message,
        t(Lang::En, Key::NetCacertUnreadable)
            .with("path", junk.display())
            .text()
    );
    // The authority that signed the server's certificate is trusted: it works.
    assert!(
        http(&server, true, None, &quick())
            .get_json(&server.url("/x"))
            .is_ok()
    );
    // Another authority replaces the built-in list and the first: the same server is now unknown.
    let other = common::fixtures().join("ca2.pem");
    let h = Http::new("127.0.0.1", Some(&other), None, &quick(), Lang::En, false).unwrap();
    assert!(matches!(
        h.get_json(&server.url("/x")).unwrap_err(),
        HttpError::Net(NetProblem::UnknownIssuer(_))
    ));
}

#[test]
fn hostile_answers_are_bounded_a_huge_error_body_a_longer_body_a_shorter_one_a_slow_drip() {
    let big_error = vec![b'x'; 5 << 20];
    let server = Server::start(
        "good",
        Arc::new(move |r: &Req| match r.path.as_str() {
            "/huge-error" => full(500, &[("Content-Type", "application/json")], &big_error),
            "/longer" => Reply::Raw(Box::new(|w| {
                // Says 10 bytes, sends a megabyte.
                let _ = w.write_all(
                    b"HTTP/1.1 200 X\r\nContent-Length: 10\r\nConnection: close\r\n\r\n",
                );
                let _ = w.write_all(&vec![b'y'; 1 << 20]);
            })),
            "/shorter" => Reply::Raw(Box::new(|w| {
                let _ = w.write_all(
                    b"HTTP/1.1 200 X\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nshort",
                );
            })),
            "/drip" => Reply::Raw(Box::new(|w| {
                let _ = w.write_all(
                    b"HTTP/1.1 200 X\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n",
                );
                for _ in 0..200 {
                    if w.write_all(b"z").is_err() || w.flush().is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })),
            "/unbounded" => Reply::Raw(Box::new(|w| {
                let _ = w.write_all(b"HTTP/1.1 200 X\r\nConnection: close\r\n\r\n");
                let chunk = vec![b'u'; 64 * 1024];
                for _ in 0..4096 {
                    if w.write_all(&chunk).is_err() {
                        return;
                    }
                }
            })),
            _ => full(200, &[], b"{}"),
        }),
    );
    let dir = scratch("hostile");
    let h = http(&server, true, None, &quick());
    // A huge error body: read up to a bound, then the status speaks.
    let e = h.get_json(&server.url("/huge-error")).unwrap_err();
    assert!(
        matches!(&e, HttpError::Server(a) if a.status == 500 && a.error.is_none()),
        "{e:?}"
    );
    // A body longer than it said: only what it said is read (the rest is not taken).
    let saved = h
        .get_file(
            &server.url("/longer"),
            &dir.join("longer"),
            None,
            1 << 30,
            &mut nothing,
        )
        .unwrap();
    assert_eq!(
        saved,
        Saved::Saved {
            etag: None,
            bytes: 10
        }
    );
    // A body shorter than it said: cut short, the partial file is kept for the next run, the destination does not exist.
    let e = h
        .get_file(
            &server.url("/shorter"),
            &dir.join("shorter"),
            None,
            1 << 30,
            &mut nothing,
        )
        .unwrap_err();
    assert!(
        matches!(e, HttpError::Net(NetProblem::CutShort(_))),
        "{e:?}"
    );
    assert!(!dir.join("shorter").exists());
    // A body with no length, over the bound the caller allows: dropped, the partial removed.
    let e = h
        .get_file(
            &server.url("/unbounded"),
            &dir.join("unbounded"),
            None,
            1 << 20,
            &mut nothing,
        )
        .unwrap_err();
    assert_eq!(e, HttpError::Net(NetProblem::TooLong));
    assert!(!dir.join("unbounded").exists() && !dir.join("unbounded.partial").exists());
    // A slow drip: the whole-call bound ends it.
    let slow = Timeouts {
        total: Duration::from_secs(2),
        idle: Duration::from_secs(1),
        ..quick()
    };
    let h = http(&server, true, None, &slow);
    let started = std::time::Instant::now();
    let e = h
        .get_file(
            &server.url("/drip"),
            &dir.join("drip"),
            None,
            1 << 30,
            &mut nothing,
        )
        .unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(
        matches!(
            e,
            HttpError::Net(NetProblem::Timeout(_) | NetProblem::CutShort(_))
        ),
        "{e:?}"
    );
}

#[test]
fn an_upload_has_an_exact_content_length_and_is_streamed_from_the_file() {
    let server = Server::start(
        "good",
        Arc::new(|r: &Req| {
            full(
                201,
                &[("Content-Type", "application/json")],
                format!(
                    r#"{{"received":{},"declared":{}}}"#,
                    r.body.len(),
                    r.header("content-length").unwrap_or("-")
                )
                .as_bytes(),
            )
        }),
    );
    let dir = scratch("post");
    let file = dir.join("artifact");
    std::fs::write(&file, body(123_456)).unwrap();
    let h = http(&server, true, Some("t0ken"), &quick());
    let prefix = b"PLANETER-PROPOSE-1\n\0\0\0\0\0\0\0\x02{}".to_vec();
    let posted = h
        .post_framed(&server.url("/propose"), &prefix, &file, &mut |_, _| {})
        .unwrap();
    assert_eq!(posted.status, 201);
    let v: serde_json::Value = serde_json::from_slice(&posted.body).unwrap();
    let want = 123_456 + prefix.len();
    assert_eq!(v["received"], want);
    assert_eq!(v["declared"], want);
    let seen = server.seen();
    assert!(seen[0].body.starts_with(&prefix));
    assert_eq!(&seen[0].body[prefix.len()..], &body(123_456)[..]);
    assert_eq!(seen[0].header("authorization"), Some("Bearer t0ken"));
}
