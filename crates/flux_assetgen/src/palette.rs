//! The shared industrial palette. Every building pulls from these so the set
//! reads as one factory, not a grab bag.

use crate::canvas::{Rgba, rgb};

pub const CONCRETE: Rgba = rgb(164, 162, 156);
pub const CONCRETE_DARK: Rgba = rgb(118, 116, 112);
pub const STEEL: Rgba = rgb(138, 148, 160);
pub const STEEL_DARK: Rgba = rgb(92, 100, 112);
pub const ROOF: Rgba = rgb(84, 100, 122);
pub const RUST: Rgba = rgb(168, 104, 60);
pub const COPPER: Rgba = rgb(198, 120, 58);
pub const WOOD: Rgba = rgb(146, 110, 72);
pub const PAD_CONCRETE: Rgba = rgb(104, 103, 100);

pub const GLOW_CYAN: Rgba = rgb(110, 230, 255);
pub const GLOW_ORANGE: Rgba = rgb(255, 150, 60);
pub const GLOW_RED: Rgba = rgb(255, 72, 52);
pub const LAMP_GREEN: Rgba = rgb(96, 238, 120);
pub const LAMP_AMBER: Rgba = rgb(255, 192, 64);
pub const LAMP_RED: Rgba = rgb(255, 80, 70);
pub const WINDOW_LIT: Rgba = rgb(255, 214, 130);
pub const WINDOW_DARK: Rgba = rgb(42, 48, 58);
pub const STEAM: Rgba = rgb(235, 240, 245);
pub const SMOKE: Rgba = rgb(96, 96, 102);
pub const FIRE: Rgba = rgb(255, 168, 64);
