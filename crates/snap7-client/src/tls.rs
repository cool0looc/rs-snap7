use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::error::Error;

pub type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

/// Build a `rustls` `ClientConfig` with webpki system roots.
/// If `extra_ca_der` is provided, it is added as a trusted CA certificate.
pub fn make_tls_config(
    extra_ca_der: Option<&[u8]>,
) -> std::result::Result<Arc<ClientConfig>, Error> {
    // Install the ring crypto provider if no process-level provider has been set yet.
    // `install_default` returns an error when already installed; we ignore that case.
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();

    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(ca_bytes) = extra_ca_der {
        let ca_cert = tokio_rustls::rustls::pki_types::CertificateDer::from(ca_bytes.to_vec());
        root_store.add(ca_cert).map_err(|e| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid CA cert: {e}"),
            ))
        })?;
    }
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Connect a TLS stream to `addr` with SNI `server_name`.
pub async fn tls_connect(
    addr: SocketAddr,
    server_name: &str,
    extra_ca_der: Option<&[u8]>,
) -> std::result::Result<TlsStream, Error> {
    let config = make_tls_config(extra_ca_der)?;
    let connector = TlsConnector::from(config);
    let tcp = TcpStream::connect(addr).await.map_err(Error::Io)?;
    let server_name = ServerName::try_from(server_name.to_string()).map_err(|e| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid server name: {e}"),
        ))
    })?;
    connector.connect(server_name, tcp).await.map_err(Error::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_config_builds_with_system_roots() {
        let _cfg = make_tls_config(None).unwrap();
    }

    #[test]
    fn tls_config_server_name_parses() {
        let name = rustls::pki_types::ServerName::try_from("plc.example.com".to_string());
        assert!(name.is_ok());
    }
}
