//! `AgentElementMakingTests`.

use std::sync::Mutex;

use async_trait::async_trait;
use jutsu_core::{AccountElementPage, ElementImage, HistoryCursor, ProviderError};
use url::Url;

use super::*;
use crate::agents::request_log::AgentRequestLog;

/// Publishes an element with a new ID, still being checked, unless `fails`; `element_images` finds it checked.
#[derive(Default)]
struct MakingElements {
    fails: bool,
    created: Mutex<Vec<String>>,
    fetched: Mutex<Vec<String>>,
}

#[async_trait]
impl AccountElements for MakingElements {
    fn owner(&self) -> Option<String> {
        Some("user_1".to_owned())
    }
    async fn elements(&self, _after: Option<HistoryCursor>) -> Result<AccountElementPage, PortError> {
        Ok(AccountElementPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }
    async fn element_images(&self, element: AccountElement) -> Result<AccountElement, PortError> {
        self.fetched.lock().unwrap().push(element.id.clone());
        Ok(AccountElement {
            id: element.id,
            name: "Mara-2".to_owned(),
            category: Some(ElementCategory::Prop),
            thumbnail_url: None,
            images: vec![ElementImage {
                url: Url::parse("https://cdn.example.com/1.png").unwrap(),
                media: None,
            }],
            status: Some("completed".to_owned()),
        })
    }
    async fn create_element(
        &self,
        name: String,
        category: ElementCategory,
        images: Vec<RemoteAsset>,
    ) -> Result<AccountElement, PortError> {
        if self.fails {
            return Err(ProviderError::Server { status: 500 }.into());
        }
        let ids: Vec<String> = images.iter().filter_map(|image| image.id.clone()).collect();
        self.created
            .lock()
            .unwrap()
            .push(format!("{name} {} {ids:?}", category.raw_value()));
        Ok(AccountElement {
            id: Uuid::new_v4().to_string(),
            name: format!("{name}-2"),
            category: Some(category),
            thumbnail_url: None,
            images: Vec::new(),
            status: Some("processing".to_owned()),
        })
    }
}

fn fields() -> CreateElementCommand {
    CreateElementCommand {
        request_id: "el-1".to_owned(),
        fingerprint: "a".repeat(64),
        name: " Mara ".to_owned(),
        category: Some("prop".to_owned()),
        images: vec![
            "/tmp/inbox/1/face.png".to_owned(),
            "/tmp/inbox/2/side.png".to_owned(),
        ],
    }
}

fn images() -> Vec<LocalImage> {
    fields()
        .images
        .iter()
        .map(|path| LocalImage::new(crate::testing::file_url(path)))
        .collect()
}

fn refusals() -> Refusals<AgentError> {
    Refusals {
        conflict: AgentError::request_conflict(),
        unreadable: AgentError::agents_off(),
        never_queued: AgentError::bad_request("never made"),
    }
}

fn making(requests: &Arc<Gate>, source: Arc<MakingElements>) -> AgentElementMaking {
    AgentElementMaking {
        requests: requests.clone(),
        source,
        upload: Arc::new(|image: LocalImage| {
            Box::pin(async move {
                let mut asset = RemoteAsset::new(Url::parse("https://cdn.example.com/u").unwrap());
                asset.id = image
                    .file_url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(str::to_owned);
                Ok(asset)
            })
        }),
        refusals: refusals(),
    }
}

fn gate() -> Arc<Gate> {
    AgentRequestGate::new(AgentRequestLog::new(None))
}

#[tokio::test]
async fn an_element_is_made_once_and_its_request_sent_again_answers_with_it() {
    let requests = gate();
    let source = Arc::new(MakingElements::default());
    let read = Arc::new(Mutex::new(0));
    let discarded = Mutex::new(0);
    let create = || {
        let read = read.clone();
        let discard = || *discarded.lock().unwrap() += 1;
        let making = making(&requests, source.clone());
        async move {
            making
                .create(
                    &fields(),
                    move || {
                        *read.lock().unwrap() += 1;
                        Ok(images())
                    },
                    discard,
                )
                .await
        }
    };

    let made = create().await.unwrap();
    assert_eq!(
        source.created.lock().unwrap().clone(),
        [r#"Mara prop ["face.png", "side.png"]"#]
    );
    let element = made.element.clone().unwrap();
    assert!(!made.known && made.jobs.is_empty());
    assert_eq!(element.name, "Mara-2");
    assert_eq!(element.category.as_deref(), Some("prop"));
    assert_eq!(element.status.as_deref(), Some("processing"));

    let again = create().await.unwrap();
    assert!(again.known);
    let known = again.element.unwrap();
    assert_eq!(known.id, element.id);
    assert_eq!(known.status.as_deref(), Some("completed"));
    assert_eq!(known.image_count, Some(1));
    assert_eq!(source.fetched.lock().unwrap().clone(), [element.id]);
    assert_eq!(source.created.lock().unwrap().len(), 1);
    assert_eq!(*read.lock().unwrap(), 1);
    assert!(*discarded.lock().unwrap() >= 2);
}

/// Sent before an answer that never came: whether it made the element isn't known, so it isn't sent again.
#[tokio::test]
async fn a_request_whose_answer_never_came_is_never_sent_again() {
    let requests = gate();
    let source = Arc::new(MakingElements::default());
    requests
        .record(
            "el-1",
            &fields().fingerprint,
            None,
            vec![Uuid::new_v4()],
            OffsetDateTime::now_utc(),
            Some(SENT_KIND),
        )
        .unwrap();
    let error = making(&requests, source.clone())
        .create(&fields(), || Ok(images()), || {})
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::FAILED);
    assert!(!error.retryable);
    assert!(error.next.unwrap().contains("new request_id"));
    assert!(source.created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_element_is_forgotten_so_the_same_request_can_be_sent_again() {
    let requests = gate();
    let failing = Arc::new(MakingElements {
        fails: true,
        ..MakingElements::default()
    });
    let error = making(&requests, failing)
        .create(&fields(), || Ok(images()), || {})
        .await
        .unwrap_err();
    assert!(error.retryable);
    let source = Arc::new(MakingElements::default());
    let made = making(&requests, source.clone())
        .create(&fields(), || Ok(images()), || {})
        .await
        .unwrap();
    assert!(!made.known);
    assert_eq!(source.created.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_same_request_id_with_other_fields_is_refused() {
    let requests = gate();
    making(&requests, Arc::new(MakingElements::default()))
        .create(&fields(), || Ok(images()), || {})
        .await
        .unwrap();
    let mut other = fields();
    other.fingerprint = "b".repeat(64);
    let error = making(&requests, Arc::new(MakingElements::default()))
        .create(&other, || Ok(images()), || {})
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::REQUEST_CONFLICT);
}
