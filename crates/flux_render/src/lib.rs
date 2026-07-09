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
    RustModule,
    Scene,
    Material,
    Animation,
    Prefab,
    Package,
    Font,
    Unknown,
}

pub fn classify(name: &str, is_dir: bool) -> AssetKind {
    if is_dir {
        return AssetKind::Folder;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".scene.json") {
        return AssetKind::Scene;
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
