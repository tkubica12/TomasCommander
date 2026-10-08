pub mod files;
pub mod journal;
pub mod platform;
pub mod preferences;
pub mod search;

#[cfg(windows)]
pub const APP_ICON_RGBA: &[u8] = include_bytes!("..\\assets\\app-icon.rgba");
