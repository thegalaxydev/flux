use std::io::Cursor;
use std::path::Path;

pub struct LoadedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl LoadedImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<LoadedImage, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(LoadedImage {
        width,
        height,
        rgba: img.into_raw(),
    })
}

pub fn load_file(path: &Path) -> Result<LoadedImage, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    decode(&bytes)
}

pub fn encode_png(img: &LoadedImage) -> Result<Vec<u8>, String> {
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .ok_or_else(|| "buffer size does not match dimensions".to_string())?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Folder,
    Image,
    Audio,
    Model,
    Script,
    LuaScript,
    /// A Luau file named `*.module.luau` — becomes a `Module` instance rather
    /// than a `Script` when dropped into the scene.
    LuaModule,
    RustModule,
    Scene,
    Material,
    Animation,
    /// A `*.tileset.json` tile palette for a `Tilemap`.
    TileSet,
    /// A `*.worldgen.json` procedural world-generation config.
    WorldGen,
    Prefab,
    Package,
    Font,
    Unknown,
    /// A plugin-registered asset type, identified by name (see
    /// [`register_asset_kind`]).
    Custom(&'static str),
}

use std::sync::RwLock;

/// flux_render's registry state — adoptable across a plugin DLL boundary (see
/// `flux_core::registries` for the pattern rationale). Holds `(suffix,
/// kind-name)` asset kinds and `(kind-name, class, target)` drop rules.
pub struct RenderRegistries {
    custom_kinds: RwLock<Vec<(&'static str, &'static str)>>,
    drops: RwLock<Vec<(&'static str, &'static str, &'static str)>>,
}

static SHARED: std::sync::OnceLock<&'static RenderRegistries> = std::sync::OnceLock::new();

fn regs() -> &'static RenderRegistries {
    SHARED.get_or_init(|| {
        Box::leak(Box::new(RenderRegistries {
            custom_kinds: RwLock::new(Vec::new()),
            drops: RwLock::new(Vec::new()),
        }))
    })
}

/// The process-wide registries — the host passes this to loaded plugins.
pub fn share_registries() -> &'static RenderRegistries {
    regs()
}

/// Adopt the host's registries (plugin entry point, before any registration).
pub fn adopt_registries(shared: &'static RenderRegistries) {
    let _ = SHARED.set(shared);
}

/// Register a plugin asset type: files ending in `suffix` classify as
/// `AssetKind::Custom(name)`.
pub fn register_asset_kind(suffix: &'static str, name: &'static str) {
    regs().custom_kinds.write().unwrap().push((suffix, name));
}

/// Register what dropping a `Custom(name)` asset into the scene creates:
/// an instance of `class` with the asset set on `prop`.
pub fn register_drop(name: &'static str, class: &'static str, prop: &'static str) {
    regs().drops.write().unwrap().push((name, class, prop));
}

/// The `(class, prop)` a dropped `Custom(name)` asset should create, if any.
pub fn drop_target(name: &str) -> Option<(&'static str, &'static str)> {
    regs()
        .drops
        .read()
        .unwrap()
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, c, p)| (*c, *p))
}

pub fn classify(name: &str, is_dir: bool) -> AssetKind {
    if is_dir {
        return AssetKind::Folder;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".scene.json") {
        return AssetKind::Scene;
    }
    if lower.ends_with(".tileset.json") {
        return AssetKind::TileSet;
    }
    if lower.ends_with(".worldgen.json") {
        return AssetKind::WorldGen;
    }
    // Plugin-registered asset suffixes (e.g. a game's catalogs).
    for (suffix, name) in regs().custom_kinds.read().unwrap().iter() {
        if lower.ends_with(suffix) {
            return AssetKind::Custom(name);
        }
    }
    // A sprite-frame library (named clips). `.spriteframes` is the user-facing
    // extension; `.frames.json` is still recognized for older assets.
    if lower.ends_with(".spriteframes") || lower.ends_with(".frames.json") {
        return AssetKind::Animation;
    }
    // `*.module.luau` (or `.lua`) is a Module; a plain `*.luau` is a Script.
    if lower.ends_with(".module.luau") || lower.ends_with(".module.lua") {
        return AssetKind::LuaModule;
    }
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tga" => AssetKind::Image,
        "wav" | "mp3" | "ogg" | "flac" => AssetKind::Audio,
        "obj" | "gltf" | "glb" | "fbx" => AssetKind::Model,
        "luau" | "lua" => AssetKind::LuaScript,
        "rs" => AssetKind::RustModule,
        "mat" => AssetKind::Material,
        "anim" => AssetKind::Animation,
        "prefab" => AssetKind::Prefab,
        "fluxpkg" | "zip" => AssetKind::Package,
        "ttf" | "otf" => AssetKind::Font,
        _ => AssetKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetKind, classify};

    #[test]
    fn module_files_are_distinguished_from_scripts() {
        assert_eq!(classify("main.luau", false), AssetKind::LuaScript);
        assert_eq!(classify("main.lua", false), AssetKind::LuaScript);
        assert_eq!(classify("balance.module.luau", false), AssetKind::LuaModule);
        assert_eq!(classify("Balance.Module.LUAU", false), AssetKind::LuaModule);
        assert_eq!(classify("util.module.lua", false), AssetKind::LuaModule);
        // A folder or non-luau file is unaffected.
        assert_eq!(classify("scripts", true), AssetKind::Folder);
        assert_eq!(classify("hero.png", false), AssetKind::Image);
    }
}
