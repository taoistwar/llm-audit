use std::net::SocketAddr;
use std::path::PathBuf;

pub struct TlsPemPaths {
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
}

/// 当 `TLS_CERT_PATH` 与 `TLS_KEY_PATH` 均为非空时，使用 Rustls 以 HTTPS 监听 `BIND_ADDR`。
pub fn tls_pem_paths() -> Option<TlsPemPaths> {
    let cert = std::env::var("TLS_CERT_PATH")
        .ok()
        .filter(|s| !s.is_empty())?;
    let key = std::env::var("TLS_KEY_PATH")
        .ok()
        .filter(|s| !s.is_empty())?;
    Some(TlsPemPaths {
        cert_pem: cert.into(),
        key_pem: key.into(),
    })
}

pub fn get_bind_addr() -> SocketAddr {
    let bind = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:19001".to_string());
    let addr: SocketAddr = bind
        .parse()
        .unwrap_or_else(|e| panic!("invalid BIND_ADDR {bind:?}: {e}"));
    addr
}