use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Body, Method};
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_util::io::ReaderStream;
use url::Url;
use uuid::Uuid;

use super::errors;
use super::wire::{
    EstimateResponse, ModelEndpoint, Preset, PresetCatalog, StatusResponse, SubmitResponse, SubmittedRequest,
    UploadTarget,
};
use crate::core::{
    GenerationStatus, HiggsfieldError, HttpResponse, ImageStatus, is_https, send, sorted_json,
};

/// REST client for `api.higgsfield.ai`. Every failure surfaces as `HiggsfieldError`.
#[derive(Clone)]
pub struct HiggsfieldApiClient {
    base_url: Url,
    key_id: String,
    http: reqwest::Client,
    authorization: String,
}

impl HiggsfieldApiClient {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.higgsfield.ai";

    /// A client of `DEFAULT_BASE_URL` on a new `reqwest` client.
    pub fn new(key_id: impl Into<String>, secret: &str) -> Self {
        let key_id = key_id.into();
        Self {
            authorization: format!("Key {key_id}:{secret}"),
            key_id,
            base_url: Self::default_base_url(),
            http: reqwest::Client::new(),
        }
    }

    /// The production API.
    ///
    /// # Panics
    ///
    /// Never: `DEFAULT_BASE_URL` is a constant that parses.
    #[allow(clippy::expect_used)] // A constant that parses.
    #[must_use]
    pub fn default_base_url() -> Url {
        Url::parse(Self::DEFAULT_BASE_URL).expect("the default base URL parses")
    }

    /// The same client against another host, as tests point it at a stub.
    #[must_use]
    pub fn with_base_url(mut self, base_url: Url) -> Self {
        self.base_url = base_url;
        self
    }

    /// The same client sending through `http`.
    #[must_use]
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    // MARK: Uploads

    /// Asks for a presigned PUT URL and the public URL the file will have once uploaded.
    pub async fn upload_target(&self, content_type: &str) -> Result<UploadTarget, HiggsfieldError> {
        let body = sorted_json(&serde_json::json!({ "content_type": content_type }));
        let response = self
            .call(self.request(Method::POST, "files/generate-upload-url", Some(&body)))
            .await?;
        decode(&response)
    }

    /// Streams the file to a presigned URL. The signature is the authorization, so no key is sent.
    /// `upload_url` comes from the server and must be https; any other fails with `Server { status: 200 }`, as an
    /// upload target the client can't act on.
    pub async fn put(
        &self,
        file: &Path,
        content_type: &str,
        upload_url: &Url,
        headers: &HashMap<String, String>,
    ) -> Result<(), HiggsfieldError> {
        if !is_https(upload_url) {
            tracing::error!("Refused an upload URL that isn't https");
            return Err(HiggsfieldError::Server { status: 200 });
        }
        let mut header_map = HeaderMap::new();
        if let Ok(value) = HeaderValue::from_str(content_type) {
            header_map.insert(CONTENT_TYPE, value);
        }
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) = (HeaderName::from_str(name), HeaderValue::from_str(value)) {
                header_map.insert(name, value);
            }
        }
        let request = file_put(&self.http, upload_url.clone(), header_map, file).await?;
        let response = send(request).await?;
        // Storage errors (e.g. 403 for an expired signature) mean nothing about the API key or balance.
        if !response.is_success() {
            log_failure("PUT", "presigned upload", &response);
            return Err(HiggsfieldError::Server {
                status: response.status,
            });
        }
        Ok(())
    }

    // MARK: Jobs

    /// The price in USD of what `submit` would send for the same request. Charges nothing.
    /// `body` is the endpoint's own (`ModelEndpoint`).
    pub async fn estimate(
        &self,
        endpoint: ModelEndpoint,
        body: &impl Serialize,
    ) -> Result<Decimal, HiggsfieldError> {
        let body = encode(body)?;
        let path = format!("estimate/{}", endpoint.raw_value());
        let response = self.call(self.request(Method::POST, &path, Some(&body))).await?;
        let estimate: EstimateResponse = decode(&response)?;
        let usd = Decimal::from_str(&estimate.usd).or_else(|_| Decimal::from_scientific(&estimate.usd));
        match usd {
            Ok(usd) if usd >= Decimal::ZERO => Ok(usd),
            _ => Err(HiggsfieldError::Server {
                status: response.status,
            }),
        }
    }

    pub async fn submit(
        &self,
        endpoint: ModelEndpoint,
        body: &impl Serialize,
        idempotency_key: Uuid,
    ) -> Result<SubmittedRequest, HiggsfieldError> {
        let body = encode(body)?;
        let request = self
            .request(Method::POST, endpoint.raw_value(), Some(&body))
            .header("Idempotency-Key", idempotency_key.hyphenated().to_string());
        let response = self.call(request).await?;
        let submitted: SubmitResponse = decode(&response)?;
        Ok(SubmittedRequest {
            initial_status: submitted.initial_status(),
            request_id: submitted.request_id,
            correlation_id: correlation_id(&response),
            status_url: submitted.status_url,
            cancel_url: submitted.cancel_url,
        })
    }

    /// `status_url` is the one the submit answer returned, used only on the API's own host.
    pub async fn status(
        &self,
        request_id: &str,
        status_url: Option<&Url>,
    ) -> Result<GenerationStatus, HiggsfieldError> {
        let url = self.trusted_url(status_url, &format!("requests/{request_id}/status"));
        let response = self.call(self.request_to(Method::GET, url, None)).await?;
        Ok(decode::<StatusResponse>(&response)?.generation_status())
    }

    /// `status` of an image model's request, which finishes with `images[].url`.
    pub async fn image_status(
        &self,
        request_id: &str,
        status_url: Option<&Url>,
    ) -> Result<ImageStatus, HiggsfieldError> {
        let url = self.trusted_url(status_url, &format!("requests/{request_id}/status"));
        let response = self.call(self.request_to(Method::GET, url, None)).await?;
        Ok(decode::<StatusResponse>(&response)?.image_status())
    }

    /// Succeeds only while the job is queued; afterwards the API answers 400 (`HiggsfieldError::Validation`).
    pub async fn cancel(&self, request_id: &str, cancel_url: Option<&Url>) -> Result<(), HiggsfieldError> {
        let url = self.trusted_url(cancel_url, &format!("requests/{request_id}/cancel"));
        self.call(self.request_to(Method::POST, url, None)).await?;
        Ok(())
    }

    /// `returned` when it is HTTPS on the API's own host and port, so the key never goes anywhere else;
    /// otherwise the documented path under `base_url`.
    pub fn trusted_url(&self, returned: Option<&Url>, path: &str) -> Url {
        let fallback = append_path(&self.base_url, path);
        let Some(returned) = returned else { return fallback };
        let same_host = returned.host_str().map(str::to_ascii_lowercase)
            == self.base_url.host_str().map(str::to_ascii_lowercase);
        if returned.scheme().eq_ignore_ascii_case("https")
            && same_host
            && returned.port() == self.base_url.port()
        {
            return returned.clone();
        }
        tracing::error!(
            "Ignored a returned URL outside {}; using {path}",
            self.base_url.host_str().unwrap_or_default()
        );
        fallback
    }

    pub async fn presets(&self) -> Result<Vec<Preset>, HiggsfieldError> {
        let path = format!("models/{}/presets", ModelEndpoint::Restyle.raw_value());
        let response = self.call(self.request(Method::GET, &path, None)).await?;
        let catalog: PresetCatalog = decode(&response)?;
        Ok(catalog
            .items
            .into_iter()
            .map(|item| Preset {
                id: item.id,
                name: item.name,
                preview_url: item.preview_url,
            })
            .collect())
    }

    /// Onboarding check with a free authenticated call. `false` means the key was refused;
    /// other failures (network, outages) are errors because they say nothing about the key.
    pub async fn validate_key(&self) -> Result<bool, HiggsfieldError> {
        // Only the status counts: a catalog item of a new shape says nothing about the key.
        let path = format!("models/{}/presets", ModelEndpoint::Restyle.raw_value());
        match self.call(self.request(Method::GET, &path, None)).await {
            Ok(_) => Ok(true),
            Err(HiggsfieldError::Unauthorized) => Ok(false),
            Err(error) => Err(error),
        }
    }

    // MARK: Transport

    fn request(&self, method: Method, path: &str, body: Option<&[u8]>) -> reqwest::RequestBuilder {
        self.request_to(method, append_path(&self.base_url, path), body)
    }

    /// Carries the key: `url` must be under `base_url` or come from `trusted_url`.
    fn request_to(&self, method: Method, url: Url, body: Option<&[u8]>) -> reqwest::RequestBuilder {
        let request = self
            .http
            .request(method, url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json");
        match body {
            Some(body) => request
                .header("Content-Type", "application/json")
                .body(body.to_vec()),
            None => request,
        }
    }

    /// Sends an API request and fails with the mapped error for any non-2xx answer.
    async fn call(&self, request: reqwest::RequestBuilder) -> Result<HttpResponse, HiggsfieldError> {
        let (method, path) = request
            .try_clone()
            .and_then(|clone| clone.build().ok())
            .map(|built| (built.method().to_string(), built.url().path().to_owned()))
            .unwrap_or_default();
        let response = send(request).await?;
        if let Some(error) = errors::error(response.status, &response.body, response.header("Retry-After")) {
            log_failure(&method, &path, &response);
            return Err(error);
        }
        Ok(response)
    }
}

/// Prints without the secret, which the `Authorization` value carries.
impl fmt::Debug for HiggsfieldApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HiggsfieldApiClient")
            .field("key_id", &self.key_id)
            .field("base_url", &self.base_url.as_str())
            .finish_non_exhaustive()
    }
}

/// `base` with `path` appended as its last components, as `URL.appendingPathComponent` does.
pub(crate) fn append_path(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    let joined = format!(
        "{}/{}",
        base.path().trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    url.set_path(&joined);
    url
}

/// A PUT of `file`'s bytes, streamed rather than read into memory, with its length: presigned storage refuses
/// a chunked body. A file that can't be opened is a transport failure.
pub(crate) async fn file_put(
    http: &reqwest::Client,
    url: Url,
    mut headers: HeaderMap,
    file: &Path,
) -> Result<reqwest::RequestBuilder, HiggsfieldError> {
    let opened = tokio::fs::File::open(file)
        .await
        .map_err(|_| HiggsfieldError::Network)?;
    let length = opened
        .metadata()
        .await
        .map_err(|_| HiggsfieldError::Network)?
        .len();
    headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
    let body = Body::wrap_stream(ReaderStream::new(opened));
    Ok(http.put(url).headers(headers).body(body))
}

fn correlation_id(response: &HttpResponse) -> Option<String> {
    response.header("X-Correlation-ID").map(str::to_owned)
}

fn encode(body: &impl Serialize) -> Result<Vec<u8>, HiggsfieldError> {
    let value = serde_json::to_value(body)
        .map_err(|_| HiggsfieldError::validation("The request has a value that can't be sent"))?;
    Ok(sorted_json(&value))
}

/// A 2xx body we can't read means the API changed shape; nothing the user can fix.
fn decode<T: DeserializeOwned>(response: &HttpResponse) -> Result<T, HiggsfieldError> {
    serde_json::from_slice(&response.body).map_err(|_| HiggsfieldError::Server {
        status: response.status,
    })
}

fn log_failure(method: &str, path: &str, response: &HttpResponse) {
    tracing::error!(
        "{method} {path} → {}, correlation {}",
        response.status,
        response.header("X-Correlation-ID").unwrap_or("none")
    );
}
