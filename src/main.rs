#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use eframe::egui;
use std::{path::PathBuf, time::Instant};
use tomas_commander::files::Scope;

fn main() {
    if let Err(error) = run() {
        tomas_commander::platform::show_error(&error.to_string());
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let mut initial =
        std::env::current_dir().map_err(|e| format!("Read working directory: {e}"))?;
    let mut fixture = None;
    let mut settings = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--root") => {
                initial = PathBuf::from(arguments.next().ok_or("--root requires a path")?)
            }
            Some("--fixture-root") => {
                let root = PathBuf::from(arguments.next().ok_or("--fixture-root requires a path")?);
                initial = root.clone();
                fixture = Some(root);
            }
            Some("--settings") => {
                settings = Some(PathBuf::from(
                    arguments.next().ok_or("--settings requires a path")?,
                ))
            }
            _ => return Err(format!("Unknown argument: {}", argument.to_string_lossy()).into()),
        }
    }
    let scope = match fixture {
        Some(root) => Scope::fixture(&root)?,
        None => Scope::unrestricted(),
    };
    initial = scope.resolve(&initial)?;
    let settings = match settings {
        Some(path) => path,
        None => {
            let root = std::env::var_os("LOCALAPPDATA")
                .ok_or("LOCALAPPDATA is not defined; pass --settings")?;
            PathBuf::from(root)
                .join("TomasCommander")
                .join("preferences.txt")
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 800.0])
            .with_min_inner_size([880.0, 560.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "TomasCommander - Ledger",
        options,
        Box::new(move |creation| {
            Ok(Box::new(app::Ledger::new(
                creation, initial, scope, settings, started,
            )))
        }),
    )?;
    Ok(())
}
