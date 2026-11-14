//! The account's credit history and uploads in the app's terms.

use higgsfield::mcp::account::{AccountAsset, AssetKind, CreditTransaction};
use jutsu_core::{
    CreditEntry, CreditEntryKind, MotionPreset, PixelSize, UploadKind, UploadedFile, decimal_shortest_of,
};

/// A credit transaction as the history lists it. A transaction's credits are read from JSON, so always finite.
pub(crate) fn entry(transaction: &CreditTransaction) -> CreditEntry {
    CreditEntry {
        date: transaction.date,
        credits: decimal_shortest_of(transaction.credits).unwrap_or_default(),
        kind: CreditEntryKind::from_raw(&transaction.action).unwrap_or(CreditEntryKind::Other),
        title: transaction.title.clone(),
    }
}

pub(crate) fn asset_kind(kind: UploadKind) -> AssetKind {
    match kind {
        UploadKind::Image => AssetKind::Image,
        UploadKind::Video => AssetKind::Video,
    }
}

/// The size of a width and height that are both known.
pub(crate) fn pixel_size(width: Option<i64>, height: Option<i64>) -> Option<PixelSize> {
    width
        .zip(height)
        .map(|(width, height)| PixelSize::new(width, height))
}

pub(crate) fn file(asset: &AccountAsset) -> UploadedFile {
    UploadedFile {
        id: asset.id.clone(),
        kind: if asset.kind == AssetKind::Video {
            UploadKind::Video
        } else {
            UploadKind::Image
        },
        url: asset.url.clone(),
        thumbnail_url: asset.thumbnail_url.clone(),
        pixel_size: pixel_size(asset.width, asset.height),
        duration: asset.duration,
        created_at: asset.created_at,
    }
}

/// Swift's `Int(exactly:)` of a whole number: `None` for one that isn't finite or doesn't fit.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the value is whole and checked to be inside i64 first"
)]
pub(crate) fn whole_number(value: f64) -> Option<i64> {
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    (value.is_finite() && value.fract() == 0.0 && (-LIMIT..LIMIT).contains(&value)).then_some(value as i64)
}

/// An uploaded video as a clip of the Motion Library: it plays itself on hover and is named by its length.
pub(crate) fn clip(file: &UploadedFile) -> MotionPreset {
    let name = file
        .duration
        .and_then(|duration| whole_number(duration.round()))
        .map_or_else(|| "Upload".to_owned(), |seconds| format!("Upload · {seconds} s"));
    MotionPreset {
        id: format!("upload:{}", file.id),
        name,
        video_url: file.url.clone(),
        thumbnail_url: file.thumbnail_url.clone(),
        preview_url: Some(file.url.clone()),
        example_mode: None,
    }
}
