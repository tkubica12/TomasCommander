#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_ui;
mod app;

use eframe::egui;
use std::{path::PathBuf, time::Instant};
use tomas_commander::files::Scope;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--ai-worker") {
        if tomas_commander::ai::worker().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("--ai-check" | "--ai-smoke" | "--ai-check-session")
    ) {
        if let Err(error) = ai_cli() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = run() {
        tomas_commander::platform::show_error(&error.to_string());
        std::process::exit(1);
    }

    fn ai_cli() -> Result<(), String> {
        use tomas_commander::ai::{self, Completion, Job};
        let config_path = ai::config_path()?;
        let config = ai::Config::load(&config_path)?;
        let prompt = (std::env::args().nth(1).as_deref() == Some("--ai-smoke"))
            .then(|| "Reply with exactly: Synthetic Foundry connection OK.".to_owned());
        let benchmark = std::env::args().nth(1).as_deref() == Some("--ai-check-session");
        let mut session = ai::Session::default();
        for trial in 0..if benchmark { 4 } else { 1 } {
            let completion = session.run(
                Job {
                    config: config.clone(),
                    prompt: prompt.clone(),
                    config_path: config_path.clone(),
                    recheck: trial == 0,
                },
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )?;
            if let Completion::Error(error) = completion {
                return Err(error);
            }
            println!(
                "{}",
                serde_json::to_string(&completion).map_err(|_| "Could not encode smoke result.")?
            );
        }
        session.clear()?;
        Ok(())
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
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 800.0])
        .with_min_inner_size(app::MIN_WINDOW_SIZE);
    #[cfg(windows)]
    {
        viewport = viewport.with_icon(egui::IconData {
            rgba: tomas_commander::APP_ICON_RGBA.to_vec(),
            width: 128,
            height: 128,
        });
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Tomas Commander",
        options,
        Box::new(move |creation| {
            Ok(Box::new(app::Ledger::new(
                creation, initial, scope, settings, started,
            )))
        }),
    )?;
    Ok(())
}
