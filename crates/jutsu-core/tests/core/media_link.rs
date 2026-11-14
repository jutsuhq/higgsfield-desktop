//! `MediaLinkTests.swift`.

use jutsu_core::*;
use url::Url;

use crate::common::*;

#[test]
fn takes_one_https_link_with_spaces_around() {
    let link = MediaLink::parse("  https://cdn.example.com/clip.mp4?sig=1\n").unwrap();
    assert_eq!(link.as_str(), "https://cdn.example.com/clip.mp4?sig=1");
}

#[test]
fn refuses_plain_http_and_what_is_not_a_web_link() {
    for text in [
        "",
        "clip.mp4",
        "file:///home/me/clip.mp4",
        "ftp://example.com/a.mp4",
        "https://",
        "https:///clip.mp4",
        "https://example.com/a.mp4 https://example.com/b.mp4",
        "see https://example.com/a.mp4",
    ] {
        assert_eq!(MediaLink::parse(text), Err(MediaLinkError::NotALink), "{text}");
    }
    assert_eq!(
        MediaLink::parse("http://cdn.example.com/clip.mp4"),
        Err(MediaLinkError::NotHttps)
    );
}

fn page() -> Url {
    Url::parse("https://www.youtube.com/watch?v=abc").unwrap()
}

fn mov() -> Url {
    Url::parse("https://storage.example.com/take.MOV").unwrap()
}

fn not_media(content_type: Option<&str>) -> MediaLinkError {
    MediaLinkError::NotMedia {
        content_type: content_type.map(str::to_owned),
    }
}

#[test]
fn known_types_name_the_kind_and_extension() {
    assert_eq!(
        MediaLink::classify(Some("video/mp4"), &page()),
        Ok((MediaLinkKind::Video, "mp4".into()))
    );
    assert_eq!(
        MediaLink::classify(Some("image/jpeg; charset=binary"), &page()),
        Ok((MediaLinkKind::Image, "jpg".into()))
    );
}

#[test]
fn a_generic_or_missing_type_falls_back_to_the_links_extension() {
    let expected = Ok((MediaLinkKind::Video, "mov".to_owned()));
    assert_eq!(
        MediaLink::classify(Some("application/octet-stream"), &mov()),
        expected
    );
    assert_eq!(MediaLink::classify(None, &mov()), expected);
    assert_eq!(MediaLink::classify(Some("video/x-unknown"), &mov()), expected);
}

#[test]
fn a_web_page_is_not_media_even_when_the_link_looks_like_a_file() {
    assert_eq!(
        MediaLink::classify(Some("text/html; charset=utf-8"), &page()),
        Err(not_media(Some("text/html")))
    );
    assert_eq!(
        MediaLink::classify(Some("text/html"), &mov()),
        Err(not_media(Some("text/html")))
    );
    assert_eq!(
        MediaLink::classify(Some("application/octet-stream"), &page()),
        Err(not_media(Some("application/octet-stream")))
    );
    assert_eq!(MediaLink::classify(None, &page()), Err(not_media(None)));
    assert_eq!(
        MediaLink::classify(Some("image/x-unknown"), &mov()),
        Err(not_media(Some("image/x-unknown")))
    );
}

#[test]
fn a_picture_to_show_takes_any_image_type_but_no_page() {
    assert_eq!(MediaLink::check_image(Some("image/avif")), Ok(()));
    assert_eq!(MediaLink::check_image(Some("application/octet-stream")), Ok(()));
    assert_eq!(MediaLink::check_image(None), Ok(()));
    assert_eq!(
        MediaLink::check_image(Some("text/html; charset=utf-8")),
        Err(not_media(Some("text/html")))
    );
    assert_eq!(
        MediaLink::check_image(Some("video/mp4")),
        Err(not_media(Some("video/mp4")))
    );
}

#[test]
fn the_file_is_named_after_the_link() {
    assert_eq!(MediaLink::base_name(&mov()), "take");
    assert_eq!(
        MediaLink::base_name(&Url::parse("https://example.com/").unwrap()),
        "example.com"
    );
}

fn origin() -> Url {
    Url::parse("https://cdn.example.com/clip.mp4").unwrap()
}

fn clip(
    range: (f64, f64),
    file: &str,
    origin: Option<Url>,
    size: PixelSize,
    allows_upscale: bool,
) -> SourceClip {
    let mut clip = SourceClip::new(
        file_url(&format!("/tmp/links/{file}")),
        12.0,
        size,
        ClosedRange::new(range.0, range.1),
    );
    clip.allows_upscale = allows_upscale;
    clip.remote_origin = origin;
    clip
}

fn linked(range: (f64, f64)) -> SourceClip {
    clip(
        range,
        "clip.mp4",
        Some(origin()),
        PixelSize::new(1280, 720),
        false,
    )
}

fn image(path: &str, origin: Option<Url>) -> LocalImage {
    let mut image = LocalImage::new(file_url(path));
    image.remote_origin = origin;
    image
}

/// Only the whole, untouched clip picked from a link is sent by it: trimmed, upscaled, converted or picked from a
/// file, it is uploaded.
#[test]
fn only_the_whole_untouched_clip_is_sent_by_its_link() {
    assert_eq!(linked((0.0, 12.0)).linkable_origin(), Some(&origin()));
    let size = PixelSize::new(1280, 720);
    for clip in [
        linked((2.0, 12.0)),
        linked((0.0, 5.0)),
        clip(
            (0.0, 12.0),
            "clip.mp4",
            Some(origin()),
            PixelSize::new(480, 360),
            true,
        ),
        clip((0.0, 12.0), "clip.mov", Some(origin()), size, false),
        clip((0.0, 12.0), "clip.mp4", None, size, false),
    ] {
        assert_eq!(clip.linkable_origin(), None, "{clip:?}");
    }
}

#[test]
fn images_of_types_the_api_takes_are_sent_by_their_link() {
    let png = image("/tmp/links/face.png", Some(origin()));
    let heic = image("/tmp/links/face.heic", Some(origin()));
    assert_eq!(png.linkable_origin(), Some(&origin()));
    assert_eq!(heic.linkable_origin(), None);
    assert_eq!(image("/tmp/face.png", None).linkable_origin(), None);
}

#[test]
fn records_stored_before_links_decode_without_one() {
    let json = r#"
        {"mode": "motionTransfer", "prompt": "", "resolution": "720p", "isDraft": false,
         "source": {"fileURL": "file:///tmp/a.mp4", "duration": 8, "pixelSize": {"width": 1280, "height": 720},
                    "range": [0, 8], "allowsUpscale": false},
         "references": [{"fileURL": "file:///tmp/b.png"}]}
        "#;

    let spec: JobSpec = serde_json::from_str(json).unwrap();

    assert_eq!(spec.source.unwrap().remote_origin, None);
    assert_eq!(
        spec.references
            .first()
            .and_then(|image| image.remote_origin.clone()),
        None
    );
}

#[test]
fn the_link_survives_encoding() {
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::genjutsu(),
        Some(linked((0.0, 12.0))),
        Resolution::P720,
    );
    spec.references = vec![image("/tmp/b.png", Some(origin()))];

    let decoded: JobSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();

    assert_eq!(decoded, spec);
    assert_eq!(
        decoded.source.and_then(|source| source.remote_origin),
        Some(origin())
    );
}
