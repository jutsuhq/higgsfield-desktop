//! Data files bundled with the crate, copied unchanged from the macOS app (`JutsuCoreResources`).

use crate::models_strings::MODELS_STRINGS;

/// List prices per model and resolution. Contributors update this file when Higgsfield changes pricing.
pub const PRICING_JSON: &str = include_str!("../resources/pricing.json");

/// The video models (`VideoModelCatalog`). Contributors add a model here, with its prices in `pricing.json`.
pub const MODELS_JSON: &str = include_str!("../resources/models.json");

/// The English text of `key` in the models' string catalog (`Models.xcstrings`), if it has one.
pub fn models_string(key: &str) -> Option<&'static str> {
    MODELS_STRINGS
        .binary_search_by(|(candidate, _)| (*candidate).cmp(key))
        .ok()
        .map(|index| MODELS_STRINGS[index].1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalog_is_sorted_and_found() {
        assert!(super::MODELS_STRINGS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(super::models_string("wan30Reference.title"), Some("Wan 3.0"));
        assert_eq!(super::models_string("nothing"), None);
    }
}
