//! Network policy is a closed enum; production code cannot opt into arbitrary plaintext HTTP.

use reqwest::{Client, Request, Response, redirect::Policy};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::{Host, Url};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPolicy {
    PublicHttps,
    PrivateDevice {
        allow_loopback: bool,
        allow_link_local: bool,
    },
    LoopbackDevelopment,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseBudget {
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
}
impl Default for ResponseBudget {
    fn default() -> Self {
        Self {
            max_header_bytes: 64 * 1024,
            max_body_bytes: 1024 * 1024,
        }
    }
}

pub struct SecureHttpClient {
    policy: NetworkPolicy,
    budget: ResponseBudget,
    total_timeout: Duration,
    connect_timeout: Duration,
}

#[cfg(test)]
mod tls_tests {
    use super::*;
    use std::{os::unix::fs::OpenOptionsExt, process::Stdio, sync::Arc};
    use tokio_rustls::{TlsAcceptor, rustls};

    #[tokio::test]
    async fn untrusted_certificate_is_refused_before_application_bytes() {
        // Every run has a new untrusted keypair in a private temporary directory.
        // No private key or reusable certificate is stored in the source tree.
        let directory = tempfile::tempdir().unwrap();
        for name in ["key.pem", "key.der", "cert.der"] {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.path().join(name))
                .unwrap();
        }
        for arguments in [
            vec![
                "req",
                "-x509",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-nodes",
                "-sha256",
                "-days",
                "1",
                "-subj",
                "/CN=127.0.0.1",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-keyout",
                "key.pem",
                "-outform",
                "DER",
                "-out",
                "cert.der",
            ],
            vec![
                "pkcs8", "-topk8", "-nocrypt", "-in", "key.pem", "-outform", "DER", "-out",
                "key.der",
            ],
        ] {
            assert!(
                std::process::Command::new("openssl")
                    .args(arguments)
                    .current_dir(directory.path())
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .expect("OpenSSL is required for the real TLS rejection test")
                    .success(),
                "temporary TLS fixture generation failed"
            );
        }
        let certificate = rustls::pki_types::CertificateDer::from(
            std::fs::read(directory.path().join("cert.der")).unwrap(),
        );
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(
            std::fs::read(directory.path().join("key.der")).unwrap(),
        );
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key.into())
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(3),
                TlsAcceptor::from(Arc::new(config)).accept(stream),
            )
            .await;
            assert!(
                matches!(result, Ok(Err(_))),
                "Untrusted TLS must fail during handshake"
            );
        });
        let client = SecureHttpClient::new(
            NetworkPolicy::PrivateDevice {
                allow_loopback: true,
                allow_link_local: false,
            },
            Duration::from_secs(2),
            Duration::from_secs(1),
            ResponseBudget::default(),
        )
        .unwrap();
        let error = client
            .get_bytes(Url::parse(&format!("https://{address}/")).unwrap())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http(ref error) if error.is_connect()));
        tokio::time::timeout(Duration::from_secs(4), server)
            .await
            .unwrap()
            .unwrap();
    }
}
impl SecureHttpClient {
    pub fn new(
        policy: NetworkPolicy,
        total_timeout: Duration,
        connect_timeout: Duration,
        budget: ResponseBudget,
    ) -> Result<Self, Error> {
        if total_timeout.is_zero()
            || connect_timeout.is_zero()
            || budget.max_header_bytes == 0
            || budget.max_body_bytes == 0
        {
            return Err(Error::InvalidBudget);
        }
        Ok(Self {
            policy,
            budget,
            total_timeout,
            connect_timeout,
        })
    }
    pub async fn validate_url(&self, url: &Url) -> Result<Vec<SocketAddr>, Error> {
        validate_url_structure(self.policy, url)?;
        let host = normalized_host(url)?;
        let port = url.port_or_known_default().ok_or(Error::UnsafeUrl)?;
        match self.policy {
            NetworkPolicy::PublicHttps if url.scheme() != "https" => {
                return Err(Error::HttpsRequired);
            }
            NetworkPolicy::PrivateDevice { .. } if !matches!(url.scheme(), "http" | "https") => {
                return Err(Error::UnsafeScheme);
            }
            NetworkPolicy::LoopbackDevelopment if url.scheme() != "http" => {
                return Err(Error::UnsafeScheme);
            }
            _ => {}
        }
        let addresses = tokio::time::timeout(
            self.total_timeout,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(Error::Resolve)?
        .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(Error::ResolveEmpty);
        }
        for address in &addresses {
            validate_address(self.policy, address.ip())?;
        }
        Ok(addresses)
    }
    pub async fn get_bytes(&self, url: Url) -> Result<Vec<u8>, Error> {
        tokio::time::timeout(self.total_timeout, async {
            let addresses = self.validate_url(&url).await?;
            let request = Request::new(reqwest::Method::GET, url);
            let response = self.execute_bound(request, &addresses).await?;
            bounded_response(response, self.budget).await
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Executes a fully constructed request while pinning the connection to the
    /// caller-supplied, already selected addresses. The request URL is validated
    /// again here so mutation between validation and execution cannot bypass the
    /// closed network policy. This low-level API returns headers; the caller
    /// owns bounded body consumption. Use execute_bounded for a full budget.
    pub async fn execute_bound(
        &self,
        mut request: Request,
        addresses: &[SocketAddr],
    ) -> Result<Response, Error> {
        validate_url_structure(self.policy, request.url())?;
        if addresses.is_empty() {
            return Err(Error::ResolveEmpty);
        }
        for address in addresses {
            validate_address(self.policy, address.ip())?;
        }
        let literal = match request.url().host().ok_or(Error::UnsafeUrl)? {
            Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
            Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
            Host::Domain(_) => None,
        };
        if let Some(ip) = literal {
            validate_address(self.policy, ip)?;
            if addresses
                .iter()
                .any(|address| canonical_ip(address.ip()) != canonical_ip(ip))
            {
                return Err(Error::AddressHostMismatch);
            }
        }
        let host = normalized_host(request.url())?;
        let expected_port = request
            .url()
            .port_or_known_default()
            .ok_or(Error::UnsafeUrl)?;
        if addresses
            .iter()
            .any(|address| address.port() != expected_port)
        {
            return Err(Error::AddressPortMismatch);
        }
        let limit = request
            .timeout()
            .copied()
            .unwrap_or(self.total_timeout)
            .min(self.total_timeout);
        *request.timeout_mut() = Some(limit);
        let client = self.bound_client(&host, addresses)?;
        Ok(tokio::time::timeout(limit, client.execute(request))
            .await
            .map_err(|_| Error::Timeout)??)
    }

    pub async fn execute_bounded(
        &self,
        request: Request,
        addresses: &[SocketAddr],
    ) -> Result<BoundedResponse, Error> {
        let limit = request
            .timeout()
            .copied()
            .unwrap_or(self.total_timeout)
            .min(self.total_timeout);
        tokio::time::timeout(limit, async {
            let response = self.execute_bound(request, addresses).await?;
            let status = response.status();
            let body = bounded_response(response, self.budget).await?;
            Ok(BoundedResponse { status, body })
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    fn bound_client(&self, host: &str, addresses: &[SocketAddr]) -> Result<Client, Error> {
        let builder = Client::builder()
            .timeout(self.total_timeout)
            .connect_timeout(self.connect_timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .resolve_to_addrs(host, addresses);
        Ok(builder.build()?)
    }
}

#[derive(Debug)]
pub struct BoundedResponse {
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
}

pub fn validate_url_structure(policy: NetworkPolicy, url: &Url) -> Result<(), Error> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(Error::UnsafeUrl);
    }
    let host = url.host().ok_or(Error::UnsafeUrl)?;
    if url.port_or_known_default().is_none() {
        return Err(Error::UnsafeUrl);
    }
    match policy {
        NetworkPolicy::PublicHttps if url.scheme() != "https" => Err(Error::HttpsRequired),
        NetworkPolicy::PrivateDevice { .. } if !matches!(url.scheme(), "http" | "https") => {
            Err(Error::UnsafeScheme)
        }
        NetworkPolicy::LoopbackDevelopment if url.scheme() != "http" => Err(Error::UnsafeScheme),
        NetworkPolicy::LoopbackDevelopment => match host {
            Host::Ipv4(address) if address.is_loopback() => Ok(()),
            Host::Ipv6(address) if canonical_ip(IpAddr::V6(address)).is_loopback() => Ok(()),
            Host::Domain(domain) if domain.eq_ignore_ascii_case("localhost") => Ok(()),
            _ => Err(Error::UnsafeUrl),
        },
        _ => Ok(()),
    }
}

fn normalized_host(url: &Url) -> Result<String, Error> {
    match url.host().ok_or(Error::UnsafeUrl)? {
        Host::Domain(domain) => Ok(domain.to_owned()),
        Host::Ipv4(address) => Ok(address.to_string()),
        Host::Ipv6(address) => Ok(address.to_string()),
    }
}

pub async fn bounded_response(
    response: Response,
    budget: ResponseBudget,
) -> Result<Vec<u8>, Error> {
    let header_bytes = response
        .headers()
        .iter()
        .try_fold(0usize, |total, (name, value)| {
            total
                .checked_add(name.as_str().len() + value.as_bytes().len() + 4)
                .ok_or(Error::ResponseTooLarge)
        })?;
    if header_bytes > budget.max_header_bytes {
        return Err(Error::ResponseTooLarge);
    }
    if response
        .content_length()
        .is_some_and(|n| n > budget.max_body_bytes as u64)
    {
        return Err(Error::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|size| size > budget.max_body_bytes)
        {
            return Err(Error::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(address),
        _ => address,
    }
}

fn validate_address(policy: NetworkPolicy, address: IpAddr) -> Result<(), Error> {
    let address = canonical_ip(address);
    if is_metadata(address) || address.is_unspecified() || address.is_multicast() {
        return Err(Error::ForbiddenAddress(address));
    }
    match policy {
        NetworkPolicy::PublicHttps if is_public_address(address) => Ok(()),
        NetworkPolicy::PublicHttps => Err(Error::ForbiddenAddress(address)),
        NetworkPolicy::LoopbackDevelopment if address.is_loopback() => Ok(()),
        NetworkPolicy::LoopbackDevelopment => Err(Error::ForbiddenAddress(address)),
        NetworkPolicy::PrivateDevice {
            allow_loopback,
            allow_link_local,
        } => {
            if address.is_loopback() && !allow_loopback {
                return Err(Error::ForbiddenAddress(address));
            }
            if is_link_local(address) && !allow_link_local {
                return Err(Error::ForbiddenAddress(address));
            }
            Ok(())
        }
    }
}
fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 88 && c == 99)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(v) => {
            let segments = v.segments();
            !(v.is_loopback()
                || v.is_unicast_link_local()
                || (segments[0] & 0xfe00) == 0xfc00
                || segments[..4] == [0x100, 0, 0, 0]
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || (segments[0] == 0x2001 && segments[1] == 0x0002))
        }
    }
}
fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_link_local(),
        IpAddr::V6(v) => v.is_unicast_link_local(),
    }
}
fn is_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.octets() == [169, 254, 169, 254],
        IpAddr::V6(v) => v.segments() == [0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254],
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network operation exceeded its total deadline")]
    Timeout,
    #[error("bound address does not match the request IP literal")]
    AddressHostMismatch,
    #[error("HTTPS is required by this network policy")]
    HttpsRequired,
    #[error("URL scheme is not allowed by this network policy")]
    UnsafeScheme,
    #[error("URL contains forbidden components")]
    UnsafeUrl,
    #[error("address is forbidden by this network policy: {0}")]
    ForbiddenAddress(IpAddr),
    #[error("network policy budget is invalid")]
    InvalidBudget,
    #[error("response exceeds its configured budget")]
    ResponseTooLarge,
    #[error("DNS resolution returned no addresses")]
    ResolveEmpty,
    #[error("validated address port does not match the request URL")]
    AddressPortMismatch,
    #[error("DNS resolution failed: {0}")]
    Resolve(std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn invalid_bindings_fail_before_connecting() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        for (policy, host, binding) in [
            (NetworkPolicy::PublicHttps, "127.0.0.1", "8.8.8.8"),
            (NetworkPolicy::PublicHttps, "[::ffff:127.0.0.1]", "8.8.8.8"),
            (NetworkPolicy::LoopbackDevelopment, "127.0.0.1", "127.0.0.2"),
            (
                NetworkPolicy::LoopbackDevelopment,
                "[::ffff:127.0.0.1]",
                "127.0.0.2",
            ),
        ] {
            let client = SecureHttpClient::new(
                policy,
                Duration::from_secs(1),
                Duration::from_secs(1),
                ResponseBudget::default(),
            )
            .unwrap();
            let scheme = if policy == NetworkPolicy::PublicHttps {
                "https"
            } else {
                "http"
            };
            let url = Url::parse(&format!("{scheme}://{host}:{port}/")).unwrap();
            let error = client
                .execute_bound(
                    Request::new(reqwest::Method::GET, url),
                    &[SocketAddr::new(binding.parse().unwrap(), port)],
                )
                .await
                .unwrap_err();
            assert!(matches!(
                error,
                Error::ForbiddenAddress(_) | Error::AddressHostMismatch
            ));
        }
        let client = SecureHttpClient::new(
            NetworkPolicy::LoopbackDevelopment,
            Duration::from_secs(1),
            Duration::from_secs(1),
            ResponseBudget::default(),
        )
        .unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert!(matches!(
            client
                .execute_bound(
                    Request::new(reqwest::Method::GET, url),
                    &[SocketAddr::from(([127, 0, 0, 1], 0))]
                )
                .await,
            Err(Error::AddressPortMismatch)
        ));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        for address in [
            "100::",
            "100::1",
            "100::ffff:ffff:ffff:ffff",
            "::ffff:10.0.0.1",
        ] {
            assert!(
                validate_address(NetworkPolicy::PublicHttps, address.parse().unwrap()).is_err(),
                "{address}"
            );
        }
        for address in ["100:0:0:1::", "ff::ffff", "::ffff:8.8.8.8"] {
            assert!(
                validate_address(NetworkPolicy::PublicHttps, address.parse().unwrap()).is_ok(),
                "{address}"
            );
        }
    }

    #[tokio::test]
    async fn total_deadline_includes_body_and_caps_request_override() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buffer = [0; 4096];
                    assert!(stream.read(&mut buffer).await.unwrap() > 0);
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
                        .await
                        .unwrap();
                    // Wait for client closure; never provide the final byte.
                    let _ = stream.read(&mut buffer).await;
                });
            }
        });
        let client = SecureHttpClient::new(
            NetworkPolicy::LoopbackDevelopment,
            Duration::from_millis(50),
            Duration::from_secs(1),
            ResponseBudget::default(),
        )
        .unwrap();
        let url = Url::parse(&format!("http://{address}/")).unwrap();
        for bounded_request in [false, true] {
            let result = tokio::time::timeout(Duration::from_secs(1), async {
                if bounded_request {
                    let mut request = Request::new(reqwest::Method::GET, url.clone());
                    *request.timeout_mut() = Some(Duration::from_secs(60));
                    client
                        .execute_bounded(request, &[address])
                        .await
                        .map(|value| value.body)
                } else {
                    client.get_bytes(url.clone()).await
                }
            })
            .await
            .expect("platform timeout must beat the caller's 60s override");
            assert!(
                matches!(result, Err(Error::Timeout))
                    || matches!(result, Err(Error::Http(ref error)) if error.is_timeout())
            );
        }
        server.abort();
    }

    #[test]
    fn rejects_remote_http_and_metadata() {
        assert!(
            validate_address(
                NetworkPolicy::LoopbackDevelopment,
                "127.0.0.1".parse().unwrap()
            )
            .is_ok()
        );
        assert!(
            validate_address(
                NetworkPolicy::LoopbackDevelopment,
                "192.0.2.1".parse().unwrap()
            )
            .is_err()
        );
        assert!(
            validate_address(
                NetworkPolicy::PrivateDevice {
                    allow_loopback: true,
                    allow_link_local: true
                },
                "169.254.169.254".parse().unwrap()
            )
            .is_err()
        );
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(
                validate_address(NetworkPolicy::PublicHttps, address.parse().unwrap()).is_err()
            );
        }
    }
}
