//! The image compare (`LibraryViewImages.imagePair`, `LibraryViewCulling.compareVariants`) and Copy Image
//! (`LibraryImageMenu`).

use uuid::Uuid;

use super::follow_ups::{compare_id, original_version};
use super::{LibraryError, LibraryService, UrlFor};
use crate::library::compare;
use crate::library::culling::CatalogIndex;
use crate::library::image_compare::{
    CompareImage, CompareImageView, ImageComparePair, ImageCompareSides, ImageCompareView,
};
use crate::library::media::{MediaRow, in_origin, material_row};
use crate::library::variants::VariantGroups;
use crate::library::view::LibraryRequest;

const NOT_TWO_IMAGES: &str = "Compare needs two images.";
const NO_ORIGINAL: &str = "This result has no earlier version.";

/// An image's pixels, upright and at full size, for Copy Image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePixels {
    pub width: u32,
    pub height: u32,
    /// Four bytes a pixel, red, green, blue and alpha, row by row.
    pub rgba: Vec<u8>,
}

impl LibraryService {
    /// The two images picked to compare, the first on the left.
    pub async fn image_compare(
        &self,
        ids: &[Uuid],
        url_for: UrlFor<'_>,
    ) -> Result<ImageCompareView, LibraryError> {
        let materials = self.materials().await?;
        let images: Vec<CompareImage> = ids
            .iter()
            .filter_map(|id| materials.iter().find(|material| material.id == *id))
            .filter_map(material_row)
            .filter_map(|row| compare_image(&row))
            .collect();
        let [left, right] = images.as_slice() else {
            return Err(LibraryError::NotFound(NOT_TWO_IMAGES.to_owned()));
        };
        pair_view(left.clone(), right.clone(), url_for)
    }

    /// Compare with Original on a saved image: the earlier version it was made from on the left, captioned
    /// "Original", and the image on the right, captioned "New version" (`MaterialVersions.imagePair`).
    pub async fn image_compare_original(
        &self,
        id: Uuid,
        url_for: UrlFor<'_>,
    ) -> Result<ImageCompareView, LibraryError> {
        let material = self.material(id).await?;
        let Some(MediaRow::LocalImage(image)) = material_row(&material) else {
            return Err(LibraryError::NotFound(NOT_TWO_IMAGES.to_owned()));
        };
        let (original, file) = original_version(&material)
            .and_then(|version| Some((version, version.file_url.clone()?)))
            .ok_or_else(|| LibraryError::NotFound(NO_ORIGINAL.to_owned()))?;
        pair_view(
            CompareImage {
                id: compare_id(original),
                url: file,
                prompt: compare::ORIGINAL_LABEL.to_owned(),
            },
            CompareImage {
                id: image.id,
                url: image.file_url,
                prompt: compare::EDITED_LABEL.to_owned(),
            },
            url_for,
        )
    }

    /// Compare Variants on an image card: the variants of `group` the grid shows for `request`, in its order, the
    /// ones whose file is missing left out.
    pub async fn image_variants(
        &self,
        request: &LibraryRequest,
        group: Uuid,
        url_for: UrlFor<'_>,
    ) -> Result<ImageCompareView, LibraryError> {
        let materials = self.materials().await?;
        let owner = self.owner();
        let index = CatalogIndex::new(&materials);
        let rows: Vec<MediaRow> = materials
            .iter()
            .filter(|material| in_origin(material, request.origin, owner.as_deref()))
            .filter_map(material_row)
            .collect();
        let filter = &request.filter;
        let shown = filter.apply(&filter.culled(rows, &index), request.kind);
        let ids: Vec<Uuid> = shown.iter().map(MediaRow::id).collect();
        let groups = VariantGroups::new(&ids, &index);
        let images: Vec<CompareImage> = groups
            .members
            .get(&group)
            .into_iter()
            .flatten()
            .filter_map(|id| shown.iter().find(|row| row.id() == *id))
            .filter_map(compare_image)
            .collect();
        let sides = ImageCompareSides::variants(images.len())
            .ok_or_else(|| LibraryError::NotFound(NOT_TWO_IMAGES.to_owned()))?;
        Ok(ImageCompareView {
            images: images.iter().map(|image| image_view(image, url_for)).collect(),
            sides,
        })
    }

    /// Copy Image: image `id`'s pixels; `None` when its file isn't here or can't be read.
    pub async fn image_pixels(&self, id: Uuid) -> Option<ImagePixels> {
        let material = self.material(id).await.ok()?;
        let Some(MediaRow::LocalImage(image)) = material_row(&material) else {
            return None;
        };
        let path = image.file_url.to_file_path().ok().filter(|path| path.exists())?;
        let picture = match &self.tools {
            Some(tools) => jutsu_media::still::image_at(tools, &path, 0).await,
            None => jutsu_media::still::image_from_bytes(tokio::fs::read(&path).await.ok()?, 0).await,
        }?;
        Some(ImagePixels {
            width: picture.width(),
            height: picture.height(),
            rgba: picture.into_raw(),
        })
    }
}

/// The image a row shows on the compare page: a saved file that is here, or an account image by its link.
fn compare_image(row: &MediaRow) -> Option<CompareImage> {
    match row {
        MediaRow::Video(_) => None,
        MediaRow::LocalImage(image) => image
            .file_url
            .to_file_path()
            .is_ok_and(|path| path.exists())
            .then(|| CompareImage {
                id: image.id,
                url: image.file_url.clone(),
                prompt: image.prompt.clone(),
            }),
        MediaRow::RemoteImage(image) => Some(CompareImage {
            id: image.job_id,
            url: image.output_url.clone(),
            prompt: image.prompt.clone(),
        }),
    }
}

/// Two images side by side, `left` on the left; refused for the same image twice.
fn pair_view(
    left: CompareImage,
    right: CompareImage,
    url_for: UrlFor<'_>,
) -> Result<ImageCompareView, LibraryError> {
    let pair = ImageComparePair::new(left, right)
        .ok_or_else(|| LibraryError::NotFound(NOT_TWO_IMAGES.to_owned()))?;
    Ok(ImageCompareView {
        images: [&pair.left, &pair.right]
            .into_iter()
            .map(|image| image_view(image, url_for))
            .collect(),
        sides: ImageCompareSides::pair(),
    })
}

fn image_view(image: &CompareImage, url_for: UrlFor<'_>) -> CompareImageView {
    let url = if image.url.scheme() == "file" {
        image.url.to_file_path().ok().and_then(|path| url_for(&path))
    } else {
        Some(image.url.to_string())
    };
    CompareImageView {
        id: image.id.to_string(),
        title: image.title(),
        url,
    }
}
