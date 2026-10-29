//! A TLS terminator for the end-to-end tests: it listens on a loopback port with a test certificate and forwards every
//! connection, decrypted, to a plain-http planeter server on another loopback port. It is a test tool (an example, never
//! installed with the program): `tls_terminator LISTEN_PORT SERVER_PORT CERT.pem KEY.pem`.
//! It prints `listening` on stderr once it accepts connections.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [listen, server, cert, key] = &args[..] else {
        eprintln!("usage: tls_terminator LISTEN_PORT SERVER_PORT CERT.pem KEY.pem");
        std::process::exit(2);
    };
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(&std::fs::read(cert).expect("cert"))
            .collect::<Result<_, _>>()
            .expect("certificates");
    let key = PrivateKeyDer::from_pem_slice(&std::fs::read(key).expect("key")).expect("key");
    let config = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("config"),
    );
    let listener = TcpListener::bind(format!("127.0.0.1:{listen}")).expect("bind");
    eprintln!("listening");
    for conn in listener.incoming() {
        let Ok(tcp) = conn else { continue };
        let (config, server) = (config.clone(), server.clone());
        thread::spawn(move || {
            let Ok(conn) = rustls::ServerConnection::new(config) else {
                return;
            };
            let Ok(upstream) = TcpStream::connect(format!("127.0.0.1:{server}")) else {
                return;
            };
            // One thread per direction; the TLS side is shared through a mutex-free split: rustls' StreamOwned is one object,
            // so the loop below alternates short non-blocking steps instead.
            pump(rustls::StreamOwned::new(conn, tcp), upstream);
        });
    }
}

/// Copy both ways until either side ends. The TLS stream cannot be split, so both sockets run with short read timeouts and the
/// loop alternates; a request body is a few megabytes at most in the tests.
fn pump(mut tls: rustls::StreamOwned<rustls::ServerConnection, TcpStream>, mut up: TcpStream) {
    let _ = tls
        .sock
        .set_read_timeout(Some(std::time::Duration::from_millis(20)));
    let _ = up.set_read_timeout(Some(std::time::Duration::from_millis(20)));
    let mut buf = vec![0u8; 64 * 1024];
    let mut idle = 0u32;
    loop {
        let mut moved = false;
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if up.write_all(&buf[..n]).is_err() {
                    break;
                }
                moved = true;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        match up.read(&mut buf) {
            Ok(0) => {
                let _ = tls.flush();
                tls.conn.send_close_notify();
                let _ = tls.flush();
                break;
            }
            Ok(n) => {
                if tls.write_all(&buf[..n]).is_err() || tls.flush().is_err() {
                    break;
                }
                moved = true;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        idle = if moved { 0 } else { idle + 1 };
        if idle > 5_000 {
            break; // five minutes of nothing
        }
    }
}
