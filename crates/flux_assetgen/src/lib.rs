//! `flux_assetgen`: deterministic procedural art for Flux games.
//!
//! Renders isometric building sprite sheets (with `*.frames.json` animation
//! clips), a terrain atlas + tileset, and UI icons — all from code, so the
//! whole art set is reproducible, tweakable, and versionable. Run via the CLI
//! (`cargo run -p flux_assetgen -- <project_root>`) or [`generate_all`].

pub mod buildings;
pub mod canvas;
pub mod icons;
pub mod iso;
pub mod palette;
pub mod terrain;

use std::path::Path;

/// Everything a caller needs to wire the generated art into a game's catalog.
pub struct Summary {
    pub buildings: Vec<buildings::Meta>,
}

/// Generate the full art set into `<root>/art` (+ `<root>/world.tileset.json`).
pub fn generate_all(root: &Path) -> Result<Summary, String> {
    terrain::generate(root)?;
    let metas = buildings::generate(root)?;
    icons::generate(root)?;
    Ok(Summary { buildings: metas })
}
