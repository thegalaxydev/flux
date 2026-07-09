use egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use egui::load::SizedTexture;

use crate::Icons;
use crate::icon::{Icon, IconSize};
use crate::theme::IconRole;

pub struct IconWidget<'a> {
    icons: &'a Icons,
    icon: Icon,
    size: f32,
    color: Option<Color32>,
    role: IconRole,
    opacity: f32,
    rotation: f32,
    mirrored: bool,
    disabled: bool,
}

impl<'a> IconWidget<'a> {
    pub(crate) fn new(icons: &'a Icons, icon: Icon) -> Self {
        Self {
            icons,
            icon,
            size: 16.0,
            color: None,
            role: IconRole::Default,
            opacity: 1.0,
            rotation: 0.0,
            mirrored: false,
            disabled: false,
        }
    }

    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }

    pub fn size_of(mut self, size: IconSize) -> Self {
        self.size = size.px();
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn role(mut self, role: IconRole) -> Self {
        self.role = role;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn rotation_degrees(mut self, degrees: f32) -> Self {
        self.rotation = degrees.to_radians();
        self
    }

    pub fn rotation_radians(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    pub fn mirrored(mut self, mirrored: bool) -> Self {
        self.mirrored = mirrored;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn resolve_color(&self) -> Color32 {
        let base = if self.disabled {
            self.icons.theme_color(IconRole::Disabled)
        } else if let Some(color) = self.color {
            color
        } else {
            self.icons.theme_color(self.role)
        };
        base.gamma_multiply(self.opacity)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), Sense::hover());
        if ui.is_rect_visible(rect) {
            let color = self.resolve_color();
            self.draw(ui, rect, color);
        }
        response
    }

    pub fn button(self, ui: &mut Ui) -> Response {
        let box_size = self.size + 8.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
        let color = if self.disabled {
            self.icons.theme_color(IconRole::Disabled)
        } else if response.hovered() && self.color.is_none() {
            self.icons.theme_color(IconRole::Selected)
        } else {
            self.resolve_color()
        };
        if response.hovered() && !self.disabled {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
        }
        if ui.is_rect_visible(rect) {
            let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(self.size));
            self.draw(ui, icon_rect, color);
        }
        response
    }

    /// Paint the icon into an explicit rect without allocating layout space.
    /// Useful for custom widgets (e.g. tree rows) that lay out by hand.
    pub fn paint_at(self, ui: &mut Ui, rect: Rect) {
        let color = self.resolve_color();
        self.draw(ui, rect, color);
    }

    fn draw(&self, ui: &mut Ui, rect: Rect, color: Color32) {
        let ppp = ui.ctx().pixels_per_point();
        let physical_px = ((self.size * ppp).round() as u32).max(1);
        let Some(texture) = self.icons.texture(ui.ctx(), self.icon, physical_px) else {
            ui.painter().rect_stroke(
                rect,
                2.0,
                Stroke::new(1.0, color),
                StrokeKind::Inside,
            );
            return;
        };
        let sized = SizedTexture::new(texture.id(), rect.size());
        let mut image = egui::Image::new(sized).tint(color);
        if self.mirrored {
            image = image.uv(Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)));
        }
        if self.rotation != 0.0 {
            image = image.rotate(self.rotation, Vec2::splat(0.5));
        }
        image.paint_at(ui, rect);
    }
}
