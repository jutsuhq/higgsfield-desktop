//! `AspectLabelTests.swift`.

use jutsu_core::*;

#[test]
fn a_size_gets_the_frame_people_name() {
    for (width, height, label) in [
        (1920, 1080, "16:9"),
        (1460, 1100, "4:3"),
        (1456, 1092, "4:3"),
        (1024, 1024, "1:1"),
        (1536, 1024, "3:2"),
        (1080, 1350, "4:5"),
        (2048, 878, "21:9"),
        (1000, 300, "10:3"),
        (1001, 300, "21:9"),
    ] {
        assert_eq!(
            PixelSize::new(width, height).aspect_label().as_deref(),
            Some(label),
            "{width}x{height}"
        );
    }
}

#[test]
fn a_stored_ratio_is_relabelled() {
    assert_eq!(PixelSize::aspect_label_of("73:55"), "4:3");
    assert_eq!(PixelSize::aspect_label_of("16:9"), "16:9");
    assert_eq!(PixelSize::aspect_label_of("auto"), "auto");
    assert_eq!(PixelSize::new(0, 10).aspect_label(), None);
}
