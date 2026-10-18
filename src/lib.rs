pub mod ai;
pub mod answers;
pub mod archive_changes;
pub mod archives;
pub mod changes;
pub mod commands;
pub mod directory_cache;
pub mod file_agent;
pub mod file_tools;
pub mod files;
mod html;
pub mod journal;
pub mod naming;
pub mod pdf;
pub mod platform;
pub mod preferences;
pub mod recent;
pub mod search;
pub mod viewer;

#[cfg(windows)]
pub const APP_ICON_RGBA: &[u8] = include_bytes!("..\\assets\\app-icon.rgba");
