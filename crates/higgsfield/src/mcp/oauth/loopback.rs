use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::authorizer::{AuthorizationPresenter, AuthorizationRedirect, MakeUrl, state_of};
use super::callback_page::CallbackPage;
use super::redirect_waiter::RedirectWaiter;
use super::registration::ClientRegistration;
use crate::mcp::error::{McpError, OAuthError};

/// Opens a URL in the user's default browser; an `Err` with the reason ends the sign-in.
pub type OpenBrowser = Arc<dyn Fn(&Url) -> Result<(), String> + Send + Sync>;

/// The most a callback request's header may take.
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// How long one browser connection may stay open.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

/// `AuthorizationPresenter` with a loopback redirect (RFC 8252 §7.3): listens on `127.0.0.1` on the first free
/// registered port, opens the authorization URL in the user's default browser and waits, for at most `timeout`,
/// for the browser to hit `/callback` with the request's `state`. Cancelling `cancel` ends the wait.
#[derive(Clone)]
pub struct LoopbackAuthorizationPresenter {
    ports: Vec<u16>,
    timeout: Duration,
    page: CallbackPage,
    open: OpenBrowser,
    cancel: CancellationToken,
    authorization_url: Arc<Mutex<Option<Url>>>,
}

impl std::fmt::Debug for LoopbackAuthorizationPresenter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoopbackAuthorizationPresenter")
            .field("ports", &self.ports)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl LoopbackAuthorizationPresenter {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_mins(10);

    pub fn new(open: OpenBrowser) -> Self {
        Self {
            ports: ClientRegistration::LOOPBACK_PORTS.to_vec(),
            timeout: Self::DEFAULT_TIMEOUT,
            page: CallbackPage::default(),
            open,
            cancel: CancellationToken::new(),
            authorization_url: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn with_ports(mut self, ports: Vec<u16>) -> Self {
        self.ports = ports;
        self
    }

    /// How long the user has to finish in the browser; then sign-in fails with `OAuthError::TimedOut`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// What the browser tab shows after the redirect.
    #[must_use]
    pub fn with_page(mut self, page: CallbackPage) -> Self {
        self.page = page;
        self
    }

    /// Cancelling this token stops a running sign-in with `HiggsfieldError::Cancelled`.
    #[must_use]
    pub fn with_cancellation(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// The authorization URL of the running sign-in, for "Open Again" and "Copy Link" while it waits; `None`
    /// before one opened.
    pub fn authorization_url(&self) -> Option<Url> {
        self.authorization_url
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AuthorizationPresenter for LoopbackAuthorizationPresenter {
    async fn authorize(&self, make_url: MakeUrl<'_>) -> Result<AuthorizationRedirect, McpError> {
        let receiver = LoopbackReceiver::listen(&self.ports, self.page.clone()).await?;
        let redirect_uri = ClientRegistration::redirect_uri(receiver.port);
        let url = make_url(&redirect_uri)?;
        receiver.redirects.expect(state_of(&url));
        *self
            .authorization_url
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(url.clone());
        (self.open)(&url).map_err(McpError::Browser)?;
        let callback = receiver.callback(self.timeout, &self.cancel).await?;
        Ok(AuthorizationRedirect {
            callback,
            redirect_uri,
        })
    }
}

/// One listener on `127.0.0.1:port` that answers every `GET /callback` and hands the expected one over. Dropping
/// it stops the listener and its connections.
pub(crate) struct LoopbackReceiver {
    pub port: u16,
    pub redirects: Arc<RedirectWaiter>,
    stop: CancellationToken,
}

impl Drop for LoopbackReceiver {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl LoopbackReceiver {
    /// Tries `ports` in order and returns the first receiver that is listening.
    pub async fn listen(ports: &[u16], page: CallbackPage) -> Result<Self, OAuthError> {
        for &port in ports {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => return Ok(Self::serve(listener, port, page)),
                Err(error) => tracing::info!(port, %error, "loopback port unavailable"),
            }
        }
        Err(OAuthError::RedirectUnavailable)
    }

    fn serve(listener: TcpListener, port: u16, page: CallbackPage) -> Self {
        let redirects = Arc::new(RedirectWaiter::default());
        let stop = CancellationToken::new();
        let page = Arc::new(page);
        tokio::spawn({
            let (redirects, stop) = (redirects.clone(), stop.clone());
            async move {
                loop {
                    let accepted = tokio::select! {
                        () = stop.cancelled() => return,
                        accepted = listener.accept() => accepted,
                    };
                    let Ok((connection, _)) = accepted else { continue };
                    let (redirects, page, stop) = (redirects.clone(), page.clone(), stop.clone());
                    tokio::spawn(async move {
                        tokio::select! {
                            () = stop.cancelled() => {}
                            _ = tokio::time::timeout(
                                CONNECTION_TIMEOUT, handle(connection, port, &redirects, &page),
                            ) => {}
                        }
                    });
                }
            }
        });
        Self {
            port,
            redirects,
            stop,
        }
    }

    /// The redirect to `/callback` that `redirects` expects.
    pub async fn callback(&self, timeout: Duration, cancel: &CancellationToken) -> Result<Url, McpError> {
        self.redirects.wait(timeout, cancel).await
    }
}

/// TCP can split even the request line: reads up to the header's terminator, within the header limit.
async fn handle(mut connection: TcpStream, port: u16, redirects: &RedirectWaiter, page: &CallbackPage) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        if request.len() >= MAX_HEADER_BYTES {
            respond(
                &mut connection,
                "431 Request Header Fields Too Large",
                "Too large",
            )
            .await;
            return;
        }
        let room = (MAX_HEADER_BYTES - request.len()).min(buffer.len());
        match connection.read(&mut buffer[..room]).await {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    let url = request_target(&request)
        .filter(|target| target.starts_with("/callback?") || target == "/callback")
        .and_then(|target| Url::parse(&format!("http://127.0.0.1:{port}{target}")).ok());
    let Some(url) = url else {
        respond(&mut connection, "404 Not Found", "Not found").await;
        return;
    };
    if !redirects.accepts(&url) {
        respond(&mut connection, "400 Bad Request", "Not this sign-in").await;
        return;
    }
    respond(&mut connection, "200 OK", &page.html(&url)).await;
    redirects.offer(url);
}

/// The path and query of an HTTP GET request line, e.g. `/callback?code=…`.
pub(crate) fn request_target(request: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(&request[..request.len().min(8 * 1024)]).ok()?;
    let line = text.split("\r\n").next()?;
    let parts: Vec<&str> = line.split(' ').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        ["GET", target, version] if version.starts_with("HTTP/") => Some((*target).to_owned()),
        _ => None,
    }
}

async fn respond(connection: &mut TcpStream, status: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = connection.write_all(head.as_bytes()).await;
    let _ = connection.write_all(body.as_bytes()).await;
    let _ = connection.shutdown().await;
}

// Real sockets on 127.0.0.1, on ports outside the registered ones, one per test so they can run in parallel.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_target_is_read_from_the_request_line() {
        let request = b"GET /callback?code=c&state=s HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(
            request_target(request).as_deref(),
            Some("/callback?code=c&state=s")
        );
        assert_eq!(request_target(b"POST /callback HTTP/1.1\r\n\r\n"), None);
        assert_eq!(request_target(b"garbage"), None);
    }

    #[tokio::test]
    async fn a_callback_can_arrive_in_separate_tcp_writes() {
        let receiver = LoopbackReceiver::listen(&[47_708], CallbackPage::default())
            .await
            .unwrap();
        receiver.redirects.expect(Some("s".into()));
        let mut connection = TcpStream::connect(("127.0.0.1", 47_708)).await.unwrap();
        connection.write_all(b"GET /call").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        connection
            .write_all(b"back?code=c&state=s HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        let callback = receiver
            .callback(Duration::from_secs(1), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(callback.query(), Some("code=c&state=s"));
    }

    /// Stands in for the browser: the authorization server redirects to `redirect_uri` with `queries` in turn,
    /// and each answer's status is kept.
    fn browser(queries: &'static [&'static str], statuses: Arc<Mutex<Vec<u16>>>) -> OpenBrowser {
        Arc::new(move |url: &Url| {
            let redirect_uri = crate::mcp::http::query_value(url, "redirect_uri").unwrap_or_default();
            let statuses = statuses.clone();
            tokio::spawn(async move {
                for query in queries {
                    if let Ok(response) = reqwest::get(format!("{redirect_uri}?{query}")).await {
                        statuses.lock().unwrap().push(response.status().as_u16());
                    }
                }
            });
            Ok(())
        })
    }

    #[tokio::test]
    async fn browser_redirect_reaches_the_authorizer() {
        let presenter =
            LoopbackAuthorizationPresenter::new(browser(&["code=the-code&state=s"], Arc::default()))
                .with_ports(vec![47_701]);
        let make_url = |redirect_uri: &str| {
            Ok(Url::parse(&format!(
                "https://auth.example/authorize?redirect_uri={redirect_uri}"
            ))
            .unwrap())
        };
        let redirect = presenter.authorize(&make_url).await.unwrap();
        assert_eq!(redirect.redirect_uri, "http://127.0.0.1:47701/callback");
        assert_eq!(
            redirect.callback.as_str(),
            "http://127.0.0.1:47701/callback?code=the-code&state=s"
        );
        assert_eq!(
            presenter.authorization_url().unwrap().as_str(),
            "https://auth.example/authorize?redirect_uri=http://127.0.0.1:47701/callback"
        );
    }

    #[tokio::test]
    async fn a_forged_callback_is_refused_and_the_genuine_one_still_arrives() {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let presenter = LoopbackAuthorizationPresenter::new(browser(
            &["error=access_denied&state=forged", "code=the-code&state=s"],
            statuses.clone(),
        ))
        .with_ports(vec![47_707]);
        let make_url = |redirect_uri: &str| {
            Ok(Url::parse(&format!(
                "https://auth.example/authorize?state=s&redirect_uri={redirect_uri}"
            ))
            .unwrap())
        };
        let redirect = presenter.authorize(&make_url).await.unwrap();
        assert_eq!(
            redirect.callback.as_str(),
            "http://127.0.0.1:47707/callback?code=the-code&state=s"
        );
        // The genuine redirect's answer may still be on its way; the forged one came first.
        assert_eq!(statuses.lock().unwrap().first(), Some(&400));
    }

    #[tokio::test]
    async fn a_browser_that_cannot_open_ends_the_sign_in() {
        let presenter = LoopbackAuthorizationPresenter::new(Arc::new(|_: &Url| Err("no browser".to_owned())))
            .with_ports(vec![47_706]);
        let make_url = |_: &str| Ok(Url::parse("https://auth.example/authorize").unwrap());
        assert_eq!(
            presenter.authorize(&make_url).await,
            Err(McpError::Browser("no browser".into()))
        );
    }

    #[tokio::test]
    async fn a_busy_port_falls_back_to_the_next_one() {
        let _busy = LoopbackReceiver::listen(&[47_702], CallbackPage::default())
            .await
            .unwrap();
        let next = LoopbackReceiver::listen(&[47_702, 47_703], CallbackPage::default())
            .await
            .unwrap();
        assert_eq!(next.port, 47_703);
    }

    #[tokio::test]
    async fn all_ports_busy_is_reported() {
        let _busy = LoopbackReceiver::listen(&[47_704], CallbackPage::default())
            .await
            .unwrap();
        let again = LoopbackReceiver::listen(&[47_704], CallbackPage::default()).await;
        assert_eq!(again.err(), Some(OAuthError::RedirectUnavailable));
    }
}
