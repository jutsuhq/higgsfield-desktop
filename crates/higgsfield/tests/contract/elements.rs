//! `ReferenceElementsTests`: `show_reference_elements` against the tool schema of the catalog snapshot of
//! 2026-10-15. The answers are in the shape of the tools' `outputSchema`s; the snapshot has no live answer.

use higgsfield::core::HiggsfieldError;
use higgsfield::mcp::account::{
    HiggsfieldAccount, ReferenceElementCategory, ReferenceElementImage, UploadedMedia,
};
use serde_json::{Value, json};
use url::Url;

use crate::support::account::{account, calls, full_set, schema_violations};
use crate::support::catalog;
use crate::support::mcp::{McpServer, tool};
use crate::support::stub::Stub;

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.push(catalog::tool("show_reference_elements"));
    tools
}

async fn elements(
    tools: Vec<Value>,
    reply: impl Fn(&str, &Value) -> Value + Send + Sync + 'static,
) -> (HiggsfieldAccount, Stub) {
    account(McpServer::new(tools).call(reply)).await
}

/// An element as the `outputSchema` describes it: in a list, `medias` holds only the first image.
fn element(id: &str, name: &str, category: &str, status: &str, medias: &Value) -> Value {
    json!({
        "id": id, "name": name, "category": category, "description": null, "status": status,
        "medias": medias, "video_medias": [], "created_at": 1_760_000_000,
    })
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn uploads() -> Vec<UploadedMedia> {
    vec![
        UploadedMedia {
            id: "m-1".into(),
            url: url("https://upload.higgsfield.ai/user_1/1.png"),
        },
        UploadedMedia {
            id: "m-2".into(),
            url: url("https://upload.higgsfield.ai/user_1/2.png"),
        },
    ]
}

fn media_ids(images: &[ReferenceElementImage]) -> Vec<Option<&str>> {
    images.iter().map(|image| image.media_id.as_deref()).collect()
}

const ELEMENT_ID: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";

#[tokio::test]
async fn list_sends_the_action_and_reads_elements_with_their_first_image() {
    let (account, stub) = elements(tools(), |_, _| {
        json!({
            "items": [
                element("el-1", "Mara", "character_ip_verified", "completed", &json!([
                    {"id": "m-1", "url": "https://upload.higgsfield.ai/u/1.jpg", "type": "media_input"},
                ])),
                element("el-2", "Rooftop", "environment", "processing", &json!([
                    {"id": "job-9", "url": "https://cdn.higgsfield.ai/2.png", "type": "image_job"},
                ])),
                element("el-3", "Lamp", "prop", "completed", &json!([
                    {"id": "m-3", "url": "http://insecure.example.com/3.jpg", "type": "media_input"},
                ])),
                {"name": "No ID"},
            ],
            "next_cursor": 1_759_999_000,
        })
    })
    .await;

    let page = account
        .reference_elements(Some(&json!(1_760_000_500)), 300)
        .await
        .unwrap();

    assert_eq!(page.next_cursor, Some(json!(1_759_999_000)));
    let ids: Vec<_> = page.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["el-1", "el-2", "el-3"]);
    let mara = &page.items[0];
    assert_eq!(mara.name, "Mara");
    assert_eq!(mara.category, Some(ReferenceElementCategory::Character));
    assert_eq!(mara.status.as_deref(), Some("completed"));
    assert_eq!(
        mara.thumbnail_url,
        Some(url("https://upload.higgsfield.ai/u/1.jpg"))
    );
    assert_eq!(
        mara.images,
        [ReferenceElementImage {
            url: url("https://upload.higgsfield.ai/u/1.jpg"),
            media_id: Some("m-1".into())
        }]
    );
    assert_eq!(media_ids(&page.items[1].images), [Some("job-9")]);
    assert_eq!(page.items[1].status.as_deref(), Some("processing"));
    assert_eq!(page.items[2].images.len(), 0);
    assert_eq!(page.items[2].thumbnail_url, None);
    assert_eq!(
        calls(&stub).await,
        [json!({
            "name": "show_reference_elements",
            "arguments": {"action": "list", "size": 100, "cursor": 1_760_000_500},
        })]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn get_reads_the_one_item_with_all_its_images() {
    let (account, stub) = elements(tools(), |_, _| {
        json!({"items": [element(ELEMENT_ID, "Mara", "character", "completed", &json!([
            {"id": "m-1", "url": "https://upload.higgsfield.ai/u/1.jpg", "type": "media_input"},
            {"id": "m-2", "url": "https://upload.higgsfield.ai/u/2.jpg", "type": "media_input"},
        ]))]})
    })
    .await;

    let element = account.reference_element(ELEMENT_ID).await.unwrap();

    assert_eq!(media_ids(&element.images), [Some("m-1"), Some("m-2")]);
    assert_eq!(
        element.thumbnail_url,
        Some(url("https://upload.higgsfield.ai/u/1.jpg"))
    );
    assert_eq!(
        calls(&stub).await[0]["arguments"],
        json!({"action": "get", "element_id": ELEMENT_ID})
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_get_without_the_element_gives_the_servers_reason() {
    let (account, _stub) = elements(tools(), |_, _| json!({"error": "Element not found"})).await;
    assert_eq!(
        account.reference_element(ELEMENT_ID).await,
        Err(HiggsfieldError::validation("Element not found"))
    );
}

#[tokio::test]
async fn an_answer_without_items_is_unexpected() {
    let (account, _stub) = elements(tools(), |_, _| json!({"elements": []})).await;
    assert_eq!(
        account.reference_elements(None, 100).await,
        Err(HiggsfieldError::validation(
            "Unexpected result from the show_reference_elements tool"
        ))
    );
}

#[tokio::test]
async fn create_sends_the_uploads_and_reads_the_new_element() {
    let (account, stub) = elements(tools(), |_, arguments| {
        let medias = arguments.get("medias").cloned().unwrap_or_else(|| json!([]));
        json!({"element": element(ELEMENT_ID, "Mara-in-red", "character", "processing", &medias)})
    })
    .await;

    let element = account
        .create_reference_element(
            Some("  Mara in red, the main character of the series  "),
            Some(ReferenceElementCategory::Character),
            &uploads(),
        )
        .await
        .unwrap();

    assert_eq!(element.id, ELEMENT_ID);
    assert_eq!(element.category, Some(ReferenceElementCategory::Character));
    assert_eq!(media_ids(&element.images), [Some("m-1"), Some("m-2")]);
    assert_eq!(
        calls(&stub).await,
        [json!({
            "name": "show_reference_elements",
            "arguments": {
                "action": "create", "category": "character",
                // Cut to the schema's 32 characters.
                "name": "Mara in red, the main character",
                "medias": [
                    {"id": "m-1", "url": "https://upload.higgsfield.ai/user_1/1.png", "type": "media_input"},
                    {"id": "m-2", "url": "https://upload.higgsfield.ai/user_1/2.png", "type": "media_input"},
                ],
            },
        })]
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn create_without_a_name_lets_the_server_name_and_classify_it() {
    let (account, stub) = elements(
        tools(),
        |_, _| json!({"element": element("el-1", "auto-name", "prop", "completed", &json!([]))}),
    )
    .await;

    account
        .create_reference_element(Some(" "), None, &uploads()[..1])
        .await
        .unwrap();

    let arguments = calls(&stub).await[0]["arguments"].clone();
    assert_eq!(arguments.get("name"), None);
    assert_eq!(arguments["category"], "auto");
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_refused_create_gives_the_servers_reason() {
    let (account, _stub) = elements(tools(), |_, _| json!({"error": "Image failed moderation"})).await;
    assert_eq!(
        account
            .create_reference_element(
                Some("Mara"),
                Some(ReferenceElementCategory::Character),
                &uploads()
            )
            .await,
        Err(HiggsfieldError::validation("Image failed moderation"))
    );
}

#[tokio::test]
async fn a_tool_that_cant_create_is_reported_without_a_call() {
    let mut listed = full_set();
    listed.push(tool(
        "show_reference_elements",
        "Elements",
        &["action", "element_id", "cursor", "size"],
    ));
    let (account, stub) = elements(listed, |_, _| json!({})).await;
    assert_eq!(
        account
            .create_reference_element(
                Some("Mara"),
                Some(ReferenceElementCategory::Character),
                &uploads()
            )
            .await,
        Err(HiggsfieldError::ToolUnavailable("show_reference_elements".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn a_missing_tool_is_reported_without_a_call() {
    let (account, stub) = elements(full_set(), |_, _| json!({})).await;
    assert_eq!(
        account.reference_elements(None, 100).await,
        Err(HiggsfieldError::ToolUnavailable("show_reference_elements".into()))
    );
    assert_eq!(calls(&stub).await, Vec::<serde_json::Value>::new());
}
