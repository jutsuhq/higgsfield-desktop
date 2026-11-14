//! The loopback media server `<video>` and `<img>` load files from (ARCHITECTURE.md, "Playback"): `axum` on
//! 127.0.0.1 on a random port, a random token per session in every URL, and only files under the folders the app
//! registers, with Range requests, content types from the extension and no directory listing.
//!
//! A file's URL is `http://127.0.0.1:<port>/<token>/<root>/<path>`: the index of its registered folder and its
//! path inside it, one percent-encoded segment per component. Paths are canonicalized before they are compared,
//! when the URL is made and again when it is served, so a link or `..` never leads out of a folder.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeFile;

/// What a path segment keeps unescaped.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Why a file has no URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaServerError {
    #[error("The file isn't in a folder Jutsu Studio shows files from.")]
    OutsideRoots,
    #[error("The file can't be found: {0}")]
    NotFound(String),
}

#[derive(Debug, Default)]
struct Roots(RwLock<Vec<PathBuf>>);

impl Roots {
    fn get(&self, index: usize) -> Option<PathBuf> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(index)
            .cloned()
    }

    fn all(&self) -> Vec<PathBuf> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[derive(Debug)]
struct Shared {
    token: String,
    roots: Roots,
}

/// The running server. Dropping it doesn't stop it; `stop` does.
#[derive(Debug, Clone)]
pub struct MediaServer {
    address: SocketAddr,
    shared: Arc<Shared>,
    stop: CancellationToken,
}

impl MediaServer {
    /// Listens on 127.0.0.1 on a port the system picks and serves `roots`.
    pub async fn start(roots: &[PathBuf]) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            token: uuid::Uuid::new_v4().simple().to_string(),
            roots: Roots::default(),
        });
        let server = Self {
            address,
            shared: shared.clone(),
            stop: CancellationToken::new(),
        };
        for root in roots {
            server.register_root(root);
        }
        let router = Router::new().fallback(serve).with_state(shared);
        let shutdown = server.stop.clone();
        tokio::spawn(async move {
            let outcome = axum::serve(listener, router)
                .with_graceful_shutdown(async move { shutdown.cancelled().await })
                .await;
            if let Err(error) = outcome {
                tracing::error!(%error, "the media server stopped");
            }
        });
        Ok(server)
    }

    /// `http://127.0.0.1:<port>`, for the CSP and the UI.
    pub fn origin(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Serves files under `root` too. A folder that doesn't exist yet is created, so its canonical path is known.
    pub fn register_root(&self, root: &Path) {
        let canonical = std::fs::create_dir_all(root)
            .and_then(|()| root.canonicalize())
            .unwrap_or_else(|_| root.to_path_buf());
        let mut roots = self
            .shared
            .roots
            .0
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }

    /// The URL of the file at `path`, which has to be under a registered folder.
    pub fn url_for(&self, path: &Path) -> Result<String, MediaServerError> {
        let canonical = path
            .canonicalize()
            .map_err(|error| MediaServerError::NotFound(error.to_string()))?;
        let roots = self.shared.roots.all();
        let (index, relative) = roots
            .iter()
            .enumerate()
            .find_map(|(index, root)| {
                canonical
                    .strip_prefix(root)
                    .ok()
                    .map(|relative| (index, relative))
            })
            .ok_or(MediaServerError::OutsideRoots)?;
        let mut url = format!("{}/{}/{index}", self.origin(), self.shared.token);
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(MediaServerError::OutsideRoots);
            };
            url.push('/');
            url.extend(utf8_percent_encode(&name.to_string_lossy(), SEGMENT));
        }
        Ok(url)
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }
}

async fn serve(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    match resolve(&shared, request.uri().path()).await {
        Some(file) => match ServeFile::new(file).try_call(request).await {
            Ok(response) => response.map(Body::new),
            Err(error) => {
                tracing::error!(%error, "can't serve a media file");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The file a request path names: the session's token, a registered folder and a path that stays inside it after
/// links are followed, ending at a file. Everything else is `None`, answered with 404 alike.
async fn resolve(shared: &Shared, path: &str) -> Option<PathBuf> {
    let mut segments = path.strip_prefix('/')?.split('/');
    if segments.next()? != shared.token {
        return None;
    }
    let root = shared.roots.get(segments.next()?.parse().ok()?)?;
    let mut file = root.clone();
    for segment in segments {
        let name = percent_decode_str(segment).decode_utf8().ok()?;
        let mut components = Path::new(name.as_ref()).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None) => file.push(name),
            _ => return None,
        }
    }
    let canonical = tokio::fs::canonicalize(&file).await.ok()?;
    let metadata = tokio::fs::metadata(&canonical).await.ok()?;
    (canonical.starts_with(&root) && metadata.is_file()).then_some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        folder: tempfile::TempDir,
        server: MediaServer,
        http: reqwest::Client,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.stop();
        }
    }

    async fn fixture() -> Fixture {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("Jutsu Studio");
        std::fs::create_dir_all(root.join("Videos")).unwrap();
        std::fs::write(root.join("Videos").join("clip one.mp4"), b"0123456789").unwrap();
        std::fs::write(folder.path().join("secret.txt"), b"outside").unwrap();
        let server = MediaServer::start(&[root]).await.unwrap();
        Fixture {
            folder,
            server,
            http: reqwest::Client::new(),
        }
    }

    impl Fixture {
        fn clip(&self) -> PathBuf {
            self.folder
                .path()
                .join("Jutsu Studio")
                .join("Videos")
                .join("clip one.mp4")
        }

        async fn get(&self, url: &str, range: Option<&str>) -> reqwest::Response {
            let mut request = self.http.get(url);
            if let Some(range) = range {
                request = request.header("Range", range);
            }
            request.send().await.unwrap()
        }
    }

    #[tokio::test]
    async fn a_registered_file_is_served_whole_with_its_content_type() {
        let fixture = fixture().await;
        let url = fixture.server.url_for(&fixture.clip()).unwrap();
        assert!(url.starts_with(&fixture.server.origin()));
        assert!(url.ends_with("/0/Videos/clip%20one.mp4"));
        let response = fixture.get(&url, None).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "video/mp4");
        assert_eq!(response.headers()["accept-ranges"], "bytes");
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"0123456789");
    }

    #[tokio::test]
    async fn range_requests_get_206_and_the_bytes_asked_for() {
        let fixture = fixture().await;
        let url = fixture.server.url_for(&fixture.clip()).unwrap();
        let response = fixture.get(&url, Some("bytes=2-5")).await;
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers()["content-range"], "bytes 2-5/10");
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"2345");

        let tail = fixture.get(&url, Some("bytes=7-")).await;
        assert_eq!(tail.status(), 206);
        assert_eq!(tail.bytes().await.unwrap().as_ref(), b"789");

        let beyond = fixture.get(&url, Some("bytes=20-30")).await;
        assert_eq!(beyond.status(), 416);
    }

    #[tokio::test]
    async fn a_wrong_or_missing_token_is_refused() {
        let fixture = fixture().await;
        let url = fixture.server.url_for(&fixture.clip()).unwrap();
        let token = fixture.server.shared.token.clone();
        let forged = url.replace(&token, "0123456789abcdef0123456789abcdef");
        assert_eq!(fixture.get(&forged, None).await.status(), 404);
        let missing = url.replace(&format!("{token}/"), "");
        assert_eq!(fixture.get(&missing, None).await.status(), 404);
    }

    #[tokio::test]
    async fn paths_outside_the_roots_are_refused() {
        let fixture = fixture().await;
        assert_eq!(
            fixture.server.url_for(&fixture.folder.path().join("secret.txt")),
            Err(MediaServerError::OutsideRoots)
        );
        let dotted = fixture
            .folder
            .path()
            .join("Jutsu Studio")
            .join("..")
            .join("secret.txt");
        assert_eq!(
            fixture.server.url_for(&dotted),
            Err(MediaServerError::OutsideRoots)
        );
        assert!(matches!(
            fixture.server.url_for(&fixture.folder.path().join("missing.mp4")),
            Err(MediaServerError::NotFound(_))
        ));

        let base = format!("{}/{}/0", fixture.server.origin(), fixture.server.shared.token);
        for escape in [
            "/../secret.txt",
            "/%2E%2E/secret.txt",
            "/Videos%2F..%2F..%2Fsecret.txt",
        ] {
            let response = fixture.get(&format!("{base}{escape}"), None).await;
            assert_eq!(response.status(), 404, "{escape}");
        }
        assert_eq!(
            fixture.get(&format!("{base}/../../1/x"), None).await.status(),
            404
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_out_of_a_root_is_refused() {
        let fixture = fixture().await;
        let link = fixture.folder.path().join("Jutsu Studio").join("link.txt");
        std::os::unix::fs::symlink(fixture.folder.path().join("secret.txt"), &link).unwrap();
        assert_eq!(fixture.server.url_for(&link), Err(MediaServerError::OutsideRoots));
        let base = format!("{}/{}/0", fixture.server.origin(), fixture.server.shared.token);
        assert_eq!(fixture.get(&format!("{base}/link.txt"), None).await.status(), 404);
    }

    #[tokio::test]
    async fn folders_are_not_listed() {
        let fixture = fixture().await;
        let base = format!("{}/{}/0", fixture.server.origin(), fixture.server.shared.token);
        assert_eq!(fixture.get(&format!("{base}/Videos"), None).await.status(), 404);
        assert_eq!(fixture.get(&format!("{base}/"), None).await.status(), 404);
        assert_eq!(fixture.get(&base, None).await.status(), 404);
    }

    #[tokio::test]
    async fn a_folder_registered_later_is_served() {
        let fixture = fixture().await;
        let cache = fixture.folder.path().join("cache");
        let image = cache.join("preview.png");
        assert!(
            fixture
                .server
                .url_for(&fixture.folder.path().join("secret.txt"))
                .is_err()
        );
        fixture.server.register_root(&cache);
        std::fs::write(&image, b"png").unwrap();
        let url = fixture.server.url_for(&image).unwrap();
        assert!(url.contains("/1/preview.png"));
        let response = fixture.get(&url, None).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "image/png");
    }
}
