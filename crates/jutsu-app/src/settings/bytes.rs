//! Sizes and counts as Settings writes them (`AppSettings.formatBytes`, the catalog's plural variations).

/// Decimal units, as the file manager shows them: "820 KB", "12 MB", "1.43 GB"; "512 bytes" below a kilobyte.
pub fn format_bytes(bytes: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size only needs three digits here")]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < 4 {
        value /= 1000.0;
        unit += 1;
    }
    let number = match unit {
        0 => return counted(bytes, "byte", "bytes"),
        1 | 2 => format!("{:.0}", value.round()),
        _ => format!("{value:.2}"),
    };
    let suffix = ["", "KB", "MB", "GB", "TB"][unit];
    format!("{number} {suffix}")
}

/// "1 file", "12 files": the English plural variations of the string catalog.
pub(crate) fn counted(count: i64, one: &str, other: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { other })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SettingsTests.cacheSizeText`, the byte half.
    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(1), "1 byte");
        assert_eq!(format_bytes(820_400), "820 KB");
        assert_eq!(format_bytes(12_300_000), "12 MB");
        assert_eq!(format_bytes(1_430_000_000), "1.43 GB");
        assert_eq!(format_bytes(2_500_000_000_000), "2.50 TB");
    }
}
