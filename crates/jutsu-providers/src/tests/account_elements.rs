//! Port of `AccountElementsTests.swift`: the account's elements (issue #43), with the tool definition of the catalog
//! snapshot and answers in the shape of its `outputSchema`.

use jutsu_core::{
    AccountElements, ElementCategory, ElementImage, GenerationProvider, Mode, PortError, PreparedFile,
    RemoteAsset, RemoteInputs, ServerMedia,
};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;

use super::support::account::{calls, calls_of, full_set, schema_violations};
use super::support::agent_d::{inputs, names, provider, spec};
use super::support::catalog;
use super::support::mcp::{McpServer, Reply};
use super::support::provider::make_provider;
use super::support::stub::Stub;

const ELEMENT_ID: &str = "3f1c8a52-6b0e-4d7a-9a51-2f4b8c9d0e17";

fn tools() -> Vec<Value> {
    let mut tools = full_set();
    tools.push(catalog::tool("show_reference_elements"));
    tools
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn media() -> ServerMedia {
    ServerMedia {
        id: "m-1".to_owned(),
        url: url("https://cdn.example.com/1.jpg"),
        owner: Some("user_1".to_owned()),
    }
}

fn item(id: &str, name: &str, status: &str, medias: &[Value]) -> Value {
    json!({
        "id": id, "name": name, "category": "prop", "description": null,
        "status": status, "medias": medias, "video_medias": [], "created_at": 1_760_000_000,
    })
}

fn reply(name: &str, arguments: &Value) -> Value {
    let first = json!({"id": "m-1", "url": "https://cdn.example.com/1.jpg", "type": "media_input"});
    match (name, arguments["action"].as_str()) {
        ("show_reference_elements", Some("list")) => json!({
            "items": [
                item(ELEMENT_ID, "Mara", "completed", &[first]),
                item("el-2", "Checking", "ip_checking", &[]),
                item("el-3", "Refused", "nsfw", &[]),
            ],
            "next_cursor": 1_759_999_000,
        }),
        ("show_reference_elements", Some("get")) => {
            let second = json!({"id": "job-7", "url": "https://cdn.example.com/2.png", "type": "image_job"});
            json!({"items": [item(ELEMENT_ID, "Mara", "completed", &[first, second])]})
        }
        ("generate_video", _) => json!({"results": [{"id": "job-1"}]}),
        _ => json!({}),
    }
}

#[tokio::test]
async fn elements_list_usable_ones_and_get_their_images() {
    let (provider, stub) = provider(tools(), reply).await;

    let elements = provider.elements(None).await.unwrap();
    let listed = elements.items.first().unwrap().clone();
    let full = provider.element_images(listed.clone()).await.unwrap();

    // Only a completed element can be used: one still being checked, or refused, is left out.
    assert_eq!(
        elements
            .items
            .iter()
            .map(|element| element.id.as_str())
            .collect::<Vec<_>>(),
        [ELEMENT_ID]
    );
    assert_eq!(listed.category, Some(ElementCategory::Prop));
    assert_eq!(listed.thumbnail_url, Some(media().url));
    // A list names only the first image, so the element asks `get` for all of them.
    assert_eq!(listed.images, Vec::<ElementImage>::new());
    assert!(elements.next_cursor.is_some());
    let second = url("https://cdn.example.com/2.png");
    assert_eq!(
        full.images,
        vec![
            ElementImage {
                url: media().url,
                media: Some(media())
            },
            ElementImage {
                url: second.clone(),
                media: Some(ServerMedia {
                    id: "job-7".to_owned(),
                    url: second,
                    owner: Some("user_1".to_owned())
                }),
            },
        ]
    );
    let actions: Vec<Value> = calls(&stub)
        .await
        .iter()
        .map(|call| call["arguments"]["action"].clone())
        .collect();
    assert_eq!(actions, vec![json!("list"), json!("get")]);
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn an_element_image_is_sent_by_its_media_id_only_for_its_account() {
    let (provider, stub) = provider(tools(), reply).await;
    let other = ServerMedia {
        id: "m-9".to_owned(),
        url: media().url,
        owner: Some("user_2".to_owned()),
    };

    let asset = provider.server_asset(&media()).await.unwrap().unwrap();
    assert_eq!(provider.server_asset(&other).await.unwrap(), None);
    let inputs = RemoteInputs {
        video: inputs().video,
        images: vec![asset],
        audio: None,
    };
    GenerationProvider::submit(&provider, &spec(Mode::MotionTransfer), &inputs, Uuid::new_v4())
        .await
        .unwrap();

    let generate = calls_of(&stub, "generate_video").await;
    let params = &generate.first().unwrap()["arguments"]["params"];
    // The image's own media ID, as a plain reference: never the element's ID, which `medias` doesn't take.
    assert_eq!(
        params["medias"],
        json!([
            {"role": "video_references", "value": "video-1"}, {"role": "image_references", "value": "m-1"},
        ])
    );
    assert!(
        !params["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("<<<"))
    );
}

/// The snapshot's `media_upload`, `media_confirm` and `show_reference_elements`; `create` answers with the element in
/// the shape of its `outputSchema`, still being checked.
async fn publishing_server() -> Stub {
    let mut tools = full_set();
    tools.extend(["media_upload", "media_confirm", "show_reference_elements"].map(catalog::tool));
    let mcp = McpServer::new(tools).call(|name, arguments| match name {
        "media_upload" => json!({
            "uploads": [
                {
                    "media_id": "m-1", "upload_url": "https://upload.example/put/1?X-Signature=abc",
                    "url": "https://upload.higgsfield.ai/user_1/m-1.png",
                },
            ],
        }),
        "media_confirm" => json!({"results": [{"media_id": "m-1", "status": "uploaded"}]}),
        "show_reference_elements" => {
            let mut element = item(ELEMENT_ID, "Mara", "processing", &[]);
            element["category"] = json!("character");
            element["medias"] = arguments.get("medias").cloned().unwrap_or_else(|| json!([]));
            json!({"element": element})
        }
        _ => json!({}),
    });
    Stub::replying(move |request| {
        if request.method.as_str() == "PUT" {
            Reply::status(200)
        } else {
            mcp.respond(request)
        }
    })
    .await
}

#[tokio::test]
async fn a_set_is_uploaded_then_created_as_a_character_element() {
    let stub = publishing_server().await;
    let provider = make_provider(&stub, false);
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(format!("{}.png", Uuid::new_v4()));
    std::fs::write(&file, "png").unwrap();

    let asset = GenerationProvider::upload(
        &provider,
        PreparedFile {
            file_url: Url::from_file_path(&file).unwrap(),
            content_type: "image/png".to_owned(),
            byte_count: 3,
            content_hash: "h".to_owned(),
        },
    )
    .await
    .unwrap();
    let element = provider
        .create_element("Mara".to_owned(), ElementCategory::Character, vec![asset])
        .await
        .unwrap();

    assert_eq!(element.id, ELEMENT_ID);
    assert_eq!(element.category, Some(ElementCategory::Character));
    assert_eq!(element.status.as_deref(), Some("processing"));
    let uploaded = url("https://upload.higgsfield.ai/user_1/m-1.png");
    assert_eq!(
        element.images,
        vec![ElementImage {
            url: uploaded.clone(),
            media: Some(ServerMedia {
                id: "m-1".to_owned(),
                url: uploaded.clone(),
                owner: Some("user_1".to_owned())
            }),
        }]
    );
    assert_eq!(
        names(&stub).await,
        vec![
            json!("media_upload"),
            json!("media_confirm"),
            json!("show_reference_elements")
        ]
    );
    assert_eq!(
        calls(&stub).await.last().map(|call| call["arguments"].clone()),
        Some(json!({
            "action": "create", "category": "character", "name": "Mara",
            "medias": [{"id": "m-1", "url": uploaded.as_str(), "type": "media_input"}],
        }))
    );
    assert!(
        stub.requests()
            .await
            .iter()
            .any(|request| request.method.as_str() == "PUT")
    );
    assert_eq!(schema_violations(&stub).await, Vec::<String>::new());
}

#[tokio::test]
async fn an_upload_without_a_media_id_is_refused_before_anything_is_sent() {
    let (provider, stub) = provider(tools(), reply).await;
    let local = RemoteAsset::new(url("https://cdn.example.com/1.jpg"));

    assert!(matches!(
        provider
            .create_element("Mara".to_owned(), ElementCategory::Character, vec![local])
            .await,
        Err(PortError::Provider(_))
    ));
    assert!(matches!(
        provider
            .create_element("Mara".to_owned(), ElementCategory::Character, Vec::new())
            .await,
        Err(PortError::Provider(_))
    ));
    assert_eq!(calls(&stub).await, Vec::<Value>::new());
}
