//! RFC 004 part 7, T5: the reader of the upload form's body. It parses hostile input, so it is held to a list of bodies it
//! must read exactly, and a list it must refuse — at every way of cutting the body into chunks.

use planeter_web::multipart::{
    Error, Event, HEADERS_MAX, PARTS_MAX, Parser, TEXT_MAX, boundary_of,
};

const CT: &str = "multipart/form-data; boundary=XyZ-boundary";

fn text(name: &str, value: &str) -> Vec<u8> {
    format!("--XyZ-boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
        .into_bytes()
}

fn file(content: &[u8]) -> Vec<u8> {
    let mut v = b"--XyZ-boundary\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"x.archive\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    v.extend_from_slice(content);
    v.extend_from_slice(b"\r\n");
    v
}

fn body(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in parts {
        v.extend_from_slice(p);
    }
    v.extend_from_slice(b"--XyZ-boundary--\r\n");
    v
}

/// What a body reads as: the text fields, the file bytes, whether the file was seen, and whether the body ended properly.
#[derive(Debug, PartialEq, Eq)]
struct Read {
    fields: Vec<(String, String)>,
    file: Option<Vec<u8>>,
}

fn read_in_chunks(bytes: &[u8], chunk: usize) -> Result<Read, Error> {
    let mut p = Parser::new(CT)?;
    let mut out = Read {
        fields: Vec::new(),
        file: None,
    };
    for c in bytes.chunks(chunk.max(1)) {
        for ev in p.push(c)? {
            match ev {
                Event::Text { name, value } => out.fields.push((name, value)),
                Event::FileStart => out.file = Some(Vec::new()),
                Event::FileData(d) => {
                    assert!(!d.is_empty(), "no empty data events");
                    out.file.as_mut().unwrap().extend_from_slice(&d)
                }
                Event::FileEnd | Event::End => {}
            }
        }
    }
    p.finish()?;
    Ok(out)
}

fn read(bytes: &[u8]) -> Result<Read, Error> {
    read_in_chunks(bytes, bytes.len().max(1))
}

#[test]
fn a_form_is_read_the_same_however_it_is_cut_into_chunks() {
    let content: Vec<u8> = (0..3000u32).map(|i| (i * 7 % 251) as u8).collect();
    let b = body(&[
        text("csrf", "tok"),
        text("owner", "alice"),
        text("name", "app"),
        file(&content),
    ]);
    let want = Read {
        fields: vec![
            ("csrf".into(), "tok".into()),
            ("owner".into(), "alice".into()),
            ("name".into(), "app".into()),
        ],
        file: Some(content.clone()),
    };
    for chunk in [1, 2, 3, 5, 7, 16, 17, 64, 100, 999, b.len()] {
        assert_eq!(
            read_in_chunks(&b, chunk),
            Ok(Read {
                fields: want.fields.clone(),
                file: want.file.clone()
            }),
            "chunk {chunk}"
        );
    }
    // Every single place to cut once.
    for at in 0..b.len() {
        let mut p = Parser::new(CT).unwrap();
        let mut got = Read {
            fields: vec![],
            file: None,
        };
        for part in [&b[..at], &b[at..]] {
            for ev in p.push(part).unwrap() {
                match ev {
                    Event::Text { name, value } => got.fields.push((name, value)),
                    Event::FileStart => got.file = Some(vec![]),
                    Event::FileData(d) => got.file.as_mut().unwrap().extend_from_slice(&d),
                    _ => {}
                }
            }
        }
        p.finish().unwrap();
        assert_eq!(got.file.as_deref(), Some(&content[..]), "cut at {at}");
        assert_eq!(got.fields.len(), 3, "cut at {at}");
    }
}

#[test]
fn a_file_with_crlf_dashes_and_a_near_delimiter_in_it_is_read_whole() {
    let mut content = Vec::new();
    content.extend_from_slice(b"\r\n--XyZ-boundar"); // a delimiter that stops one byte short
    content.extend_from_slice(b"\r\n--\r\n---\r\n\r\n\r\n\r\n");
    content.extend_from_slice(b"\r\n--XyZ-boundaryX"); // the delimiter and then more: the reader must refuse, see below
    let b = body(&[file(&content[..content.len() - 17])]);
    let got = read_in_chunks(&b, 3).unwrap();
    assert_eq!(got.file.unwrap(), &content[..content.len() - 17]);
    // The delimiter followed by neither `--` nor CRLF is not data: it is refused.
    let bad = body(&[file(&content)]);
    assert_eq!(read(&bad), Err(Error::Framing));
}

#[test]
fn an_empty_file_and_an_empty_field_are_fine() {
    let b = body(&[text("csrf", ""), file(b"")]);
    let got = read(&b).unwrap();
    assert_eq!(got.fields, vec![("csrf".into(), "".into())]);
    assert_eq!(got.file, Some(vec![]));
}

#[test]
fn what_follows_the_closing_delimiter_is_ignored() {
    let mut b = body(&[text("a", "b")]);
    b.extend_from_slice(&vec![b'x'; 100_000]);
    assert_eq!(read(&b).unwrap().fields.len(), 1);
}

#[test]
fn a_body_that_stops_early_is_truncated_at_every_cut() {
    let b = body(&[text("csrf", "tok"), file(&vec![9u8; 500])]);
    // The CRLF after the closing delimiter is optional, so the body is whole two bytes before its end.
    for at in 0..b.len() - 2 {
        let mut p = Parser::new(CT).unwrap();
        let first = p.push(&b[..at]);
        let verdict = first.and_then(|_| p.finish());
        assert!(
            verdict.is_err(),
            "cut at {at} must not read as a whole body"
        );
    }
}

#[test]
fn the_form_must_start_with_the_delimiter() {
    let mut b = b"preamble\r\n".to_vec();
    b.extend_from_slice(&body(&[text("a", "b")]));
    assert_eq!(read(&b), Err(Error::Framing));
    assert_eq!(read(b"--XyZ-boundarX"), Err(Error::Framing));
    assert_eq!(read(b""), Err(Error::Truncated));
}

#[test]
fn the_file_is_the_last_part_and_there_is_one() {
    let after = body(&[file(b"abc"), text("late", "x")]);
    assert_eq!(read(&after), Err(Error::FileNotLast));
    let two = body(&[file(b"abc"), file(b"def")]);
    assert_eq!(read(&two), Err(Error::FileNotLast));
    // A file part under any other name is refused: only `archive` is a file.
    let other = b"--XyZ-boundary\r\nContent-Disposition: form-data; name=\"owner\"; filename=\"a\"\r\n\r\nabc\r\n--XyZ-boundary--\r\n".to_vec();
    assert_eq!(read(&other), Err(Error::FileNotLast));
}

#[test]
fn headers_and_fields_are_bounded() {
    let long_header = format!(
        "--XyZ-boundary\r\nContent-Disposition: form-data; name=\"a\"\r\nX-Pad: {}\r\n\r\nv\r\n--XyZ-boundary--\r\n",
        "p".repeat(HEADERS_MAX + 10)
    );
    assert_eq!(read(long_header.as_bytes()), Err(Error::Headers));
    // Headers that never end do not make the reader hold the body.
    let mut p = Parser::new(CT).unwrap();
    p.push(b"--XyZ-boundary\r\n").unwrap();
    assert_eq!(
        p.push(&vec![b'h'; HEADERS_MAX + 10]).map(|_| ()),
        Err(Error::Headers)
    );
    assert_eq!(
        read(&body(&[text("a", &"v".repeat(TEXT_MAX))]))
            .unwrap()
            .fields[0]
            .1
            .len(),
        TEXT_MAX
    );
    assert_eq!(
        read(&body(&[text("a", &"v".repeat(TEXT_MAX + 1))])),
        Err(Error::TextTooLong)
    );
    // A field that never ends is refused while it grows, not after the whole body is held.
    let mut p = Parser::new(CT).unwrap();
    p.push(b"--XyZ-boundary\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n")
        .unwrap();
    assert_eq!(
        p.push(&vec![b'v'; TEXT_MAX + 200]).map(|_| ()),
        Err(Error::TextTooLong)
    );
    let many: Vec<Vec<u8>> = (0..=PARTS_MAX)
        .map(|i| text(&format!("f{i}"), "x"))
        .collect();
    assert_eq!(read(&body(&many)), Err(Error::TooManyParts));
    assert!(read(&body(&many[..PARTS_MAX])).is_ok());
}

#[test]
fn a_field_that_is_not_text_is_refused() {
    let b = format!("--XyZ-boundary\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n")
        .into_bytes();
    let mut v = b;
    v.extend_from_slice(&[0xff, 0xfe, b'\r', b'\n']);
    v.extend_from_slice(b"--XyZ-boundary--\r\n");
    assert_eq!(read(&v), Err(Error::NotUtf8));
}

#[test]
fn malformed_dispositions_are_refused() {
    for head in [
        "Content-Type: text/plain",
        "Content-Disposition: attachment; name=\"a\"",
        "Content-Disposition: form-data",
        "Content-Disposition: form-data; name=a",
        "Content-Disposition: form-data; name=\"\"",
        "Content-Disposition: form-data; name=\"a b\"",
        "Content-Disposition: form-data; name=\"../x\"",
        "Content-Disposition: form-data; name=\"a\"; name=\"b\"",
        "no colon here",
    ] {
        let b = format!("--XyZ-boundary\r\n{head}\r\n\r\nv\r\n--XyZ-boundary--\r\n");
        assert_eq!(read(b.as_bytes()), Err(Error::Headers), "{head}");
    }
    // Two disposition headers.
    let b = "--XyZ-boundary\r\nContent-Disposition: form-data; name=\"a\"\r\nContent-Disposition: form-data; name=\"b\"\r\n\r\nv\r\n--XyZ-boundary--\r\n";
    assert_eq!(read(b.as_bytes()), Err(Error::Headers));
    // The header name is case-insensitive; other headers are allowed.
    let b = "--XyZ-boundary\r\ncontent-disposition: FORM-DATA; NAME=\"a\"\r\nX-Other: 1\r\n\r\nv\r\n--XyZ-boundary--\r\n";
    assert_eq!(
        read(b.as_bytes()).unwrap().fields,
        vec![("a".into(), "v".into())]
    );
}

#[test]
fn the_filename_is_never_kept() {
    let b = b"--XyZ-boundary\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"../../etc/passwd\"\r\n\r\nabc\r\n--XyZ-boundary--\r\n";
    let got = read(b).unwrap();
    assert_eq!(got.file.as_deref(), Some(&b"abc"[..]));
}

#[test]
fn the_content_type_must_be_multipart_form_data_with_a_usable_boundary() {
    assert_eq!(
        boundary_of("multipart/form-data; boundary=abc").as_deref(),
        Some("abc")
    );
    assert_eq!(
        boundary_of("Multipart/Form-Data; Boundary=\"ab\"").as_deref(),
        Some("ab")
    );
    assert_eq!(
        boundary_of("multipart/form-data;boundary=----WebKitFormBoundary7MA4YWxk").as_deref(),
        Some("----WebKitFormBoundary7MA4YWxk")
    );
    for bad in [
        "application/x-www-form-urlencoded",
        "multipart/mixed; boundary=abc",
        "multipart/form-data",
        "multipart/form-data; boundary=",
        "multipart/form-data; boundary=\"\"",
        &format!("multipart/form-data; boundary={}", "a".repeat(71)),
        "multipart/form-data; boundary=a\r\nb",
        "multipart/form-data; boundary=\"a b\"",
        "multipart/form-data; boundary=a<b",
        "",
    ] {
        assert_eq!(boundary_of(bad), None, "{bad:?}");
    }
    assert_eq!(Parser::new("text/plain").err(), Some(Error::NotMultipart));
}
