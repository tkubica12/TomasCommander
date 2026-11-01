use super::{keyboard::*, pane_ui::*, *};

mod dialogs;
mod drag;
mod frame;
mod keyboard;
mod palette;
mod pane;
mod pane_ui;
mod proposal;
mod recent_rail;
mod tabs;
mod theme;
mod usage;
mod work;

fn entry(name: &str) -> Entry {
    Entry {
        path: PathBuf::from(name),
        name: name.into(),
        directory: false,
        link: false,
        size: 1,
        modified: None,
        folded: name.to_lowercase(),
        aggregate: None,
    }
}
