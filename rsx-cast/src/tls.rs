//! TLS plumbing shared by the DXS replay client + server.

use crate::config::TlsConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::ServerConfig;
use std::fs;
use std::io;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;

pub fn build_connector(
    cfg: &TlsConfig,
) -> io::Result<TlsConnector> {
    let mut root_store = RootCertStore::empty();

    if let Some(ca_path) = &cfg.cert_path {
        let ca_pem = fs::read(ca_path)?;
        for cert in load_certs(&ca_pem)? {
            root_store.add(cert).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to add cert: {}", e),
                )
            })?;
        }
    } else {
        // TODO(TODO.md 10-DEPLOY): add webpki_roots fallback for public TLS
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TLS requires cert_path in config",
        ));
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

pub fn build_acceptor(
    cfg: &TlsConfig,
) -> io::Result<TlsAcceptor> {
    let cert_path =
        cfg.cert_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "cert_path required",
            )
        })?;
    let key_path =
        cfg.key_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "key_path required",
            )
        })?;

    let certs = load_certs(&fs::read(cert_path)?)?;
    let key = load_private_key(&fs::read(key_path)?)?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("tls config error: {}", e),
            )
        })?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn extract_server_name(
    addr: &str,
) -> io::Result<ServerName<'static>> {
    let host = addr
        .split(':')
        .next()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid address format",
            )
        })?
        .to_string();

    ServerName::try_from(host)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid server name: {}", e),
            )
        })
        .map(|name| name.to_owned())
}

fn load_certs(
    pem: &[u8],
) -> io::Result<Vec<CertificateDer<'static>>> {
    let mut cursor = io::Cursor::new(pem);
    rustls_pemfile::certs(&mut cursor)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad cert pem: {}", e),
            )
        })
}

fn load_private_key(
    pem: &[u8],
) -> io::Result<PrivateKeyDer<'static>> {
    let mut cursor = io::Cursor::new(pem);
    rustls_pemfile::private_key(&mut cursor)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad key pem: {}", e),
            )
        })?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "no private key found",
            )
        })
}
