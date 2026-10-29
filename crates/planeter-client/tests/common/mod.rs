//! A small https server for the tests: rustls on a loopback port, one thread per connection, `Connection: close`, and a handler
//! that decides each answer. It is the "TLS terminator" of the assessment, with the origin built in.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tls")
}

/// A request as the server saw it.
#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What the handler decides.
pub type RawWriter = Box<dyn FnOnce(&mut dyn Write) + Send>;

pub enum Reply {
    /// A complete answer.
    Full {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// The handler writes whatever bytes it likes (a body shorter or longer than it said, a drip, …).
    Raw(RawWriter),
}

pub fn full(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    Reply::Full {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        body: body.to_vec(),
    }
}

pub type Handler = Arc<dyn Fn(&Req) -> Reply + Send + Sync>;

pub struct Server {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Req>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn config(cert: &str) -> Arc<rustls::ServerConfig> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let dir = fixtures();
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&std::fs::read(dir.join(format!("{cert}.pem"))).unwrap())
            .collect::<Result<_, _>>()
            .unwrap();
    let key =
        PrivateKeyDer::from_pem_slice(&std::fs::read(dir.join(format!("{cert}.key"))).unwrap())
            .unwrap();
    Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap(),
    )
}

fn read_request(stream: &mut dyn Read) -> Option<Req> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 || head.len() > 64 * 1024 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.lines();
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).ok()?;
    Some(Req {
        method,
        path,
        headers,
        body,
    })
}

fn write_reply(w: &mut dyn Write, reply: Reply) {
    match reply {
        Reply::Full {
            status,
            headers,
            body,
        } => {
            let mut head = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
            if !headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            {
                head.push_str(&format!("Content-Length: {}\r\n", body.len()));
            }
            for (k, v) in headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            let _ = w.write_all(head.as_bytes());
            let _ = w.write_all(&body);
        }
        Reply::Raw(f) => f(w),
    }
    let _ = w.flush();
}

impl Server {
    /// Start a server with the named certificate (`good`, `wrong`, `exp`, `other`).
    pub fn start(cert: &str, handler: Handler) -> Server {
        let cfg = config(cert);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, halt) = (requests.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(tcp) = conn else { continue };
                let (cfg, handler, log) = (cfg.clone(), handler.clone(), log.clone());
                std::thread::spawn(move || {
                    let _ = tcp.set_read_timeout(Some(Duration::from_secs(20)));
                    let _ = tcp.set_write_timeout(Some(Duration::from_secs(20)));
                    let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                        return;
                    };
                    let mut tls = rustls::StreamOwned::new(conn, tcp);
                    let Some(req) = read_request(&mut tls) else {
                        return;
                    };
                    log.lock().unwrap().push(req.clone());
                    let reply = handler(&req);
                    write_reply(&mut tls, reply);
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                });
            }
        });
        Server {
            port,
            requests,
            stop,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    pub fn seen(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }
}

/// A scratch directory under the target directory.
pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("client-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The CA certificate file that makes `good`, `wrong` and `exp` trusted.
pub fn ca() -> PathBuf {
    fixtures().join("ca.pem")
}
