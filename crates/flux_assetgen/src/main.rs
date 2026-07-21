use std::path::PathBuf;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("projects/reactor"));

    match flux_assetgen::generate_all(&root) {
        Ok(summary) => {
            println!("art written to {}", root.join("art").display());
            println!("{:<10} {:>9} {:>14} — catalog `sprite*` fields", "building", "frame", "pivot");
            for m in &summary.buildings {
                println!(
                    "{:<10} {:>4}x{:<4} [{:.3}, {:.3}]  sprite: \"{}\"",
                    m.id, m.frame.0, m.frame.1, m.pivot.0, m.pivot.1, m.frames_asset
                );
            }
        }
        Err(e) => {
            eprintln!("assetgen failed: {e}");
            std::process::exit(1);
        }
    }
}
