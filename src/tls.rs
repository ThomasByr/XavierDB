//! Optional TLS with live certificate reload, plus the axum Listener types.
//!
//! When TLS_CERT_PATH and TLS_KEY_PATH point to valid PEM files, the server
//! listens with TLS. The certificate/key are watched; on change they are
//! re-parsed and swapped atomically — new connections use the new
//! certificate, established ones keep the old.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::serve::Listener;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

pub struct TlsState {
    pub acceptor: RwLock<Option<TlsAcceptor>>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// Load cert+key from disk and build a rustls ServerConfig.
pub fn load_server_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> Result<rustls::ServerConfig, String> {
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| format!("cannot open cert {}: {e}", cert_path.display()))?;
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("invalid certificate: {e}"))?;
    if certs.is_empty() {
        return Err("no certificate found in file".into());
    }
    let key_file = std::fs::File::open(key_path)
        .map_err(|e| format!("cannot open key {}: {e}", key_path.display()))?;
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut BufReader::new(key_file))
        .map_err(|e| format!("invalid private key: {e}"))?
        .ok_or_else(|| "no private key found in file".to_string())?;
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("cert/key mismatch: {e}"))
}

impl TlsState {
    pub fn new(cert_path: PathBuf, key_path: PathBuf) -> Result<Self, String> {
        let cfg = load_server_config(&cert_path, &key_path)?;
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        Ok(Self {
            acceptor: RwLock::new(Some(acceptor)),
            cert_path,
            key_path,
        })
    }

    /// Re-parse the current files and swap the acceptor. Old connections keep
    /// their certificate; new ones get the fresh one.
    pub fn reload(&self) -> Result<(), String> {
        let cfg = load_server_config(&self.cert_path, &self.key_path)?;
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        *self.acceptor.write().unwrap() = Some(acceptor);
        Ok(())
    }
}

/// Address type used for ConnectInfo on both plain and TLS listeners.
/// (axum only implements `Connected` for `SocketAddr` on its built-in
/// `TcpListener`, so custom listeners expose their own newtype.)
#[derive(Debug, Clone, Copy)]
pub struct MyAddr(pub std::net::SocketAddr);

/// Plain-HTTP listener (kept alongside the TLS one so both share `MyAddr`).
pub struct PlainIncoming {
    pub listener: TcpListener,
}

impl Listener for PlainIncoming {
    type Io = TcpStream;
    type Addr = MyAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((s, a)) => return (s, MyAddr(a)),
                Err(e) => {
                    eprintln!("[http] accept error: {e}");
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr().map(MyAddr)
    }
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, PlainIncoming>>
    for MyAddr
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, PlainIncoming>) -> Self {
        stream.remote_addr().clone()
    }
}

/// A Listener that accepts TCP connections and upgrades them to TLS using the
/// *current* acceptor — this is what makes hot reload possible.
pub struct TlsIncoming {
    pub listener: TcpListener,
    pub tls: Arc<TlsState>,
}

impl TlsIncoming {
    pub fn new(listener: TcpListener, tls: Arc<TlsState>) -> Self {
        Self { listener, tls }
    }
}

impl Listener for TlsIncoming {
    type Io = tokio_rustls::server::TlsStream<TcpStream>;
    type Addr = MyAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = match self.listener.accept().await {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("[tls] accept error: {e}");
                    continue;
                }
            };
            let acceptor = self.tls.acceptor.read().unwrap().clone();
            match acceptor {
                Some(a) => match a.accept(stream).await {
                    Ok(tls) => return (tls, MyAddr(addr)),
                    Err(e) => eprintln!("[tls] handshake error: {e}"),
                },
                None => {
                    // TLS was disabled at runtime; drop the connection.
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr().map(MyAddr)
    }
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, TlsIncoming>>
    for MyAddr
{
    fn connect_info(stream: axum::serve::IncomingStream<'_, TlsIncoming>) -> Self {
        stream.remote_addr().clone()
    }
}
