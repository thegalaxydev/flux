use crate::icon::Icon;
use crate::lucide_data;

#[derive(Clone, Copy)]
pub struct IconSource {
    pub id: &'static str,
    pub svg: &'static str,
}

pub trait IconProvider {
    fn name(&self) -> &str;
    fn resolve(&self, icon: Icon) -> Option<IconSource>;
}

pub struct LucideProvider;

impl LucideProvider {
    fn lucide_name(icon: Icon) -> &'static str {
        match icon {
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder-open",
            Icon::Scene => "clapperboard",
            Icon::Script => "file-code",
            Icon::LuaScript => "scroll-text",
            Icon::RustModule => "file-cog",
            Icon::Material => "palette",
            Icon::Mesh => "box",
            Icon::Texture => "image",
            Icon::Sprite => "sticker",
            Icon::Tilemap => "grid-3x3",
            Icon::Font => "type",
            Icon::Audio => "music",
            Icon::Animation => "film",
            Icon::Prefab => "blocks",
            Icon::Package => "package",
            Icon::Plugin => "puzzle",
            Icon::World => "globe",
            Icon::Entity => "boxes",
            Icon::Component => "component",
            Icon::Camera => "camera",
            Icon::Light => "lightbulb",
            Icon::Physics => "atom",
            Icon::PhysicsBody => "atom",
            Icon::ParticleSystem => "sparkles",
            Icon::UiCanvas => "layout-template",
            Icon::Ui => "layout-template",
            Icon::Settings => "settings",
            Icon::Search => "search",
            Icon::Filter => "filter",
            Icon::Play => "play",
            Icon::Pause => "pause",
            Icon::Stop => "square",
            Icon::Step => "step-forward",
            Icon::New => "file-plus",
            Icon::Open => "folder-open",
            Icon::Save => "save",
            Icon::SaveAll => "save-all",
            Icon::Undo => "undo-2",
            Icon::Redo => "redo-2",
            Icon::Cut => "scissors",
            Icon::Copy => "copy",
            Icon::Paste => "clipboard-paste",
            Icon::Duplicate => "files",
            Icon::Delete => "trash-2",
            Icon::Rename => "pencil",
            Icon::Refresh => "refresh-cw",
            Icon::Add => "plus",
            Icon::Remove => "minus",
            Icon::Close => "x",
            Icon::Download => "download",
            Icon::Upload => "upload",
            Icon::Lock => "lock",
            Icon::Unlock => "lock-open",
            Icon::Visible => "eye",
            Icon::Hidden => "eye-off",
            Icon::Terminal => "terminal",
            Icon::Console => "square-terminal",
            Icon::Profiler => "gauge",
            Icon::Explorer => "panel-left",
            Icon::Inspector => "panel-right",
            Icon::Properties => "sliders-horizontal",
            Icon::Hierarchy => "list-tree",
            Icon::AssetBrowser => "folder-tree",
            Icon::Project => "folder-kanban",
            Icon::Build => "hammer",
            Icon::Publish => "rocket",
            Icon::PackageManager => "package-search",
            Icon::Git => "git-branch",
            Icon::Transform => "move-3d",
            Icon::Rendering => "monitor",
            Icon::Scripting => "code",
            Icon::Networking => "network",
            Icon::Metadata => "tag",
            Icon::Image => "image",
            Icon::Model => "box",
            Icon::UnknownFile => "file",
            Icon::Warning => "triangle-alert",
            Icon::Error => "circle-x",
            Icon::Success => "circle-check",
            Icon::Info => "info",
            Icon::Loading => "loader-circle",
            Icon::Syncing => "refresh-cw",
            Icon::Downloading => "arrow-down-to-line",
            Icon::Uploading => "arrow-up-to-line",
            Icon::Offline => "wifi-off",
            Icon::Online => "wifi",
        }
    }
}

impl IconProvider for LucideProvider {
    fn name(&self) -> &str {
        "lucide"
    }

    fn resolve(&self, icon: Icon) -> Option<IconSource> {
        let id = Self::lucide_name(icon);
        lucide_data::svg(id).map(|svg| IconSource { id, svg })
    }
}
