mod icon;
mod lucide_data;
mod provider;
mod theme;
mod widget;

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};
use resvg::{tiny_skia, usvg};

pub use icon::{Icon, IconSize};
pub use provider::{IconProvider, IconSource, LucideProvider};
pub use theme::{IconRole, IconTheme};
pub use widget::IconWidget;

pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Icons {
    provider: Box<dyn IconProvider>,
    theme: IconTheme,
    trees: RefCell<HashMap<&'static str, usvg::Tree>>,
    textures: RefCell<HashMap<(&'static str, u32), TextureHandle>>,
}

impl Icons {
    pub fn new(provider: Box<dyn IconProvider>, theme: IconTheme) -> Self {
        Self {
            provider,
            theme,
            trees: RefCell::new(HashMap::new()),
            textures: RefCell::new(HashMap::new()),
        }
    }

    pub fn lucide() -> Self {
        Self::new(Box::new(LucideProvider), IconTheme::default())
    }

    pub fn provider_name(&self) -> String {
        self.provider.name().to_string()
    }

    pub fn theme(&self) -> IconTheme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: IconTheme) {
        self.theme = theme;
    }

    pub fn sync_theme_from(&mut self, visuals: &egui::Visuals) {
        self.theme = IconTheme::from_visuals(visuals);
    }

    pub fn icon(&self, icon: Icon) -> IconWidget<'_> {
        IconWidget::new(self, icon)
    }

    pub fn resolve(&self, icon: Icon) -> Option<IconSource> {
        self.provider.resolve(icon)
    }

    pub fn rasterize(&self, icon: Icon, physical_px: u32) -> Option<RgbaImage> {
        let src = self.provider.resolve(icon)?;
        self.ensure_tree(src);
        let trees = self.trees.borrow();
        let tree = trees.get(src.id)?;
        let px = physical_px.max(1);
        let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
        let size = tree.size();
        let transform = tiny_skia::Transform::from_scale(
            px as f32 / size.width().max(1.0),
            px as f32 / size.height().max(1.0),
        );
        resvg::render(tree, transform, &mut pixmap.as_mut());
        let mut rgba = Vec::with_capacity((px * px * 4) as usize);
        for p in pixmap.pixels() {
            let c = p.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        Some(RgbaImage {
            width: px,
            height: px,
            rgba,
        })
    }

    pub(crate) fn theme_color(&self, role: IconRole) -> Color32 {
        self.theme.color(role)
    }

    pub(crate) fn texture(&self, ctx: &Context, icon: Icon, physical_px: u32) -> Option<TextureHandle> {
        let src = self.provider.resolve(icon)?;
        let key = (src.id, physical_px);
        if let Some(handle) = self.textures.borrow().get(&key) {
            return Some(handle.clone());
        }
        let image = self.rasterize(icon, physical_px)?;
        let color_image =
            ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
        let handle = ctx.load_texture(
            format!("flux_icon:{}:{physical_px}", src.id),
            color_image,
            TextureOptions::LINEAR,
        );
        self.textures.borrow_mut().insert(key, handle.clone());
        Some(handle)
    }

    fn ensure_tree(&self, src: IconSource) {
        if self.trees.borrow().contains_key(src.id) {
            return;
        }
        let prepared = src.svg.replace("currentColor", "#ffffff");
        let options = usvg::Options::default();
        if let Ok(tree) = usvg::Tree::from_str(&prepared, &options) {
            self.trees.borrow_mut().insert(src.id, tree);
        }
    }
}
