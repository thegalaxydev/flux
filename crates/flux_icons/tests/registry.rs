use flux_icons::{Icon, IconProvider, Icons, LucideProvider};

#[test]
fn every_icon_resolves_to_a_source() {
    let provider = LucideProvider;
    for icon in Icon::ALL {
        let src = provider
            .resolve(*icon)
            .unwrap_or_else(|| panic!("{icon:?} has no source"));
        assert!(!src.svg.is_empty(), "{icon:?} svg empty");
        assert!(src.svg.contains("<svg"), "{icon:?} not an svg");
    }
}

#[test]
fn every_icon_rasterizes_non_empty() {
    let icons = Icons::lucide();
    for icon in Icon::ALL {
        let img = icons
            .rasterize(*icon, 32)
            .unwrap_or_else(|| panic!("{icon:?} failed to rasterize"));
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
        let has_pixels = img.rgba.chunks_exact(4).any(|p| p[3] > 0);
        assert!(has_pixels, "{icon:?} rendered fully transparent");
    }
}

#[test]
fn provider_is_swappable_behind_trait() {
    let provider: Box<dyn IconProvider> = Box::new(LucideProvider);
    assert_eq!(provider.name(), "lucide");
    assert!(provider.resolve(Icon::Save).is_some());
}
