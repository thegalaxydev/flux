use egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconRole {
    Default,
    Muted,
    Accent,
    Selected,
    Disabled,
    Success,
    Warning,
    Error,
    Info,
}

#[derive(Clone, Copy)]
pub struct IconTheme {
    pub default: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub selected: Color32,
    pub disabled: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub info: Color32,
}

impl IconTheme {
    pub fn dark() -> Self {
        Self {
            default: Color32::from_gray(210),
            muted: Color32::from_gray(140),
            accent: Color32::from_rgb(90, 169, 230),
            selected: Color32::from_rgb(255, 200, 60),
            disabled: Color32::from_gray(96),
            success: Color32::from_rgb(99, 198, 122),
            warning: Color32::from_rgb(224, 190, 80),
            error: Color32::from_rgb(235, 100, 100),
            info: Color32::from_rgb(90, 169, 230),
        }
    }

    pub fn light() -> Self {
        Self {
            default: Color32::from_gray(60),
            muted: Color32::from_gray(110),
            accent: Color32::from_rgb(21, 101, 192),
            selected: Color32::from_rgb(196, 120, 0),
            disabled: Color32::from_gray(170),
            success: Color32::from_rgb(46, 125, 50),
            warning: Color32::from_rgb(184, 134, 11),
            error: Color32::from_rgb(198, 40, 40),
            info: Color32::from_rgb(21, 101, 192),
        }
    }

    pub fn from_visuals(v: &egui::Visuals) -> Self {
        let mut base = if v.dark_mode { Self::dark() } else { Self::light() };
        base.default = v.text_color();
        base.muted = v.weak_text_color();
        base.accent = v.hyperlink_color;
        let sel = v.selection.stroke.color;
        if sel.a() > 0 {
            base.selected = sel;
        }
        base
    }

    pub fn color(&self, role: IconRole) -> Color32 {
        match role {
            IconRole::Default => self.default,
            IconRole::Muted => self.muted,
            IconRole::Accent => self.accent,
            IconRole::Selected => self.selected,
            IconRole::Disabled => self.disabled,
            IconRole::Success => self.success,
            IconRole::Warning => self.warning,
            IconRole::Error => self.error,
            IconRole::Info => self.info,
        }
    }
}

impl Default for IconTheme {
    fn default() -> Self {
        Self::dark()
    }
}
