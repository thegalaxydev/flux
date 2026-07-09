use std::collections::HashMap;
use std::path::Path;
use std::time::{Instant, SystemTime};

use egui::{Context, TextureHandle, TextureOptions};

struct Entry {
    handle: Option<TextureHandle>,
    mtime: Option<SystemTime>,
}

#[derive(Default)]
pub struct TextureCache {
    entries: HashMap<String, Entry>,
    last_poll: Option<Instant>,
}

impl TextureCache {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_poll = None;
    }

    pub fn get(&mut self, ctx: &Context, root: &Path, rel: &str) -> Option<TextureHandle> {
        if rel.is_empty() {
            return None;
        }
        if let Some(entry) = self.entries.get(rel) {
            return entry.handle.clone();
        }
        self.load(ctx, root, rel)
    }

    pub fn poll_hot_reload(&mut self, ctx: &Context, root: &Path) {
        let now = Instant::now();
        if let Some(last) = self.last_poll {
            if now.duration_since(last).as_secs_f32() < 0.5 {
                return;
            }
        }
        self.last_poll = Some(now);
        let keys: Vec<String> = self.entries.keys().cloned().collect();
        for rel in keys {
            let full = root.join(&rel);
            let mtime = std::fs::metadata(&full).and_then(|m| m.modified()).ok();
            if self.entries.get(&rel).map(|e| &e.mtime) != Some(&mtime) {
                self.load(ctx, root, &rel);
            }
        }
    }

    fn load(&mut self, ctx: &Context, root: &Path, rel: &str) -> Option<TextureHandle> {
        let full = root.join(rel);
        let mtime = std::fs::metadata(&full).and_then(|m| m.modified()).ok();
        let handle = match flux_render::load_file(&full) {
            Ok(img) => {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [img.width as usize, img.height as usize],
                    &img.rgba,
                );
                Some(ctx.load_texture(format!("flux_tex:{rel}"), color, TextureOptions::LINEAR))
            }
            Err(_) => None,
        };
        self.entries.insert(
            rel.to_string(),
            Entry {
                handle: handle.clone(),
                mtime,
            },
        );
        handle
    }
}
