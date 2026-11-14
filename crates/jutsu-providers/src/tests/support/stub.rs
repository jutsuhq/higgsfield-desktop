//! Stub HTTP hosts: a `wiremock` server, reached in plain HTTP or through a TLS front with the test CA's
//! `localhost` certificate, so the code paths that insist on https run against it unchanged.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use url::Url;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const CA: &[u8] = include_bytes!("../../../tests/fixtures/tls/ca.pem");
const CERT: &[u8] = include_bytes!("../../../tests/fixtures/tls/cert.pem");
const KEY: &[u8] = include_bytes!("../../../tests/fixtures/tls/key.pem");

/// A `reqwest` client that trusts only the test CA.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(CA).unwrap()])
        .build()
        .unwrap()
}

/// A stub host on its own port.
pub struct Stub {
    pub mock: MockServer,
    /// `https://localhost:<port>` for a TLS stub, else the mock's own `http://127.0.0.1:<port>`.
    pub base_url: Url,
    front: Option<JoinHandle<()>>,
    front_port: Option<u16>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        if let Some(front) = self.front.take() {
            front.abort();
        }
    }
}

impl Stub {
    /// A plain HTTP stub.
    pub async fn http() -> Self {
        let mock = MockServer::start().await;
        let base_url = Url::parse(&mock.uri()).unwrap();
        Self {
            mock,
            base_url,
            front: None,
            front_port: None,
        }
    }

    /// A stub behind TLS at `https://localhost:<port>`.
    pub async fn https() -> Self {
        let mock = MockServer::start().await;
        let target = *mock.address();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = TlsAcceptor::from(Arc::new(server_config()));
        let front = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let Ok(mut upstream) = TcpStream::connect(target).await else {
                        return;
                    };
                    let _ = tokio::io::copy_bidirectional(&mut tls, &mut upstream).await;
                });
            }
        });
        let base_url = Url::parse(&format!("https://localhost:{port}")).unwrap();
        Self {
            mock,
            base_url,
            front: Some(front),
            front_port: Some(port),
        }
    }

    /// A TLS stub answering every request with `handler`, like the Swift `StubServer { request, _ in … }`.
    pub async fn serve(handler: impl Fn(&Request) -> ResponseTemplate + Send + Sync + 'static) -> Self {
        let stub = Self::https().await;
        Mock::given(any()).respond_with(handler).mount(&stub.mock).await;
        stub
    }

    /// `base_url` with `path`.
    pub fn url(&self, path: &str) -> Url {
        self.base_url.join(path).unwrap()
    }

    pub async fn requests(&self) -> Vec<Request> {
        self.mock.received_requests().await.unwrap_or_default()
    }

    /// The requests to `url`: its host, path and query.
    pub async fn requests_to(&self, url: &str) -> Vec<Request> {
        let url = Url::parse(url).unwrap();
        self.requests()
            .await
            .into_iter()
            .filter(|request| target(request) == url)
            .collect()
    }

    /// A client trusting the test CA whose every host name reaches this TLS stub, as the Swift tests' stub
    /// sessions route real host names (`mcp.higgsfield.ai`, `clerk.higgsfield.ai`, …) to their server.
    pub fn client(&self) -> reqwest::Client {
        self.client_with_unreachable("", Arc::default())
    }

    /// `client`, except that `host` is never found, like a DNS lookup that finds nothing; `lookups` counts the
    /// requests that tried it.
    pub fn client_with_unreachable(&self, host: &str, lookups: Arc<AtomicUsize>) -> reqwest::Client {
        let port = self.front_port.expect("only a TLS stub routes host names");
        let resolver = EveryHost {
            address: SocketAddr::from(([127, 0, 0, 1], port)),
            unreachable: host.to_owned(),
            lookups,
        };
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(CA).unwrap()])
            .dns_resolver(Arc::new(resolver))
            .build()
            .unwrap()
    }
}

/// The URL a request was sent to, by its `Host` header: `https://<host><path>?<query>`.
pub fn target(request: &Request) -> Url {
    let host = header(request, "host").unwrap_or("localhost");
    let mut url = Url::parse(&format!("https://{host}")).unwrap();
    url.set_path(request.url.path());
    url.set_query(request.url.query());
    url
}

/// Resolves every host name to the stub's TLS front.
struct EveryHost {
    address: SocketAddr,
    unreachable: String,
    lookups: Arc<AtomicUsize>,
}

impl Resolve for EveryHost {
    fn resolve(&self, name: Name) -> Resolving {
        let address = self.address;
        if name.as_str() == self.unreachable {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            return Box::pin(async { Err("host not found".into()) });
        }
        Box::pin(async move { Ok(Box::new(std::iter::once(address)) as Addrs) })
    }
}

/// A form body's fields, percent-decoded.
pub fn form(request: &Request) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(&request.body).into_owned().collect()
}

fn server_config() -> rustls::ServerConfig {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let certs = vec![CertificateDer::from_pem_slice(CERT).unwrap()];
    let key = PrivateKeyDer::from_pem_slice(KEY).unwrap();
    rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap()
}

/// A URL nothing listens on: a request there gets no HTTP answer.
pub fn dead_url(path: &str) -> Url {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap()
}

/// A request's header, by any case.
pub fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers.get(name).and_then(|value| value.to_str().ok())
}

pub fn body_text(request: &Request) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

pub fn body_json(request: &Request) -> serde_json::Value {
    serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null)
}
