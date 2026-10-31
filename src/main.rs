#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_ui;
mod answers_ui;
mod app;
#[cfg(feature = "native-desktop-tests")]
mod desktop_metrics;
mod file_agent_ui;
mod text_preview_ui;
mod viewer_ui;

use eframe::egui;
use std::{path::PathBuf, time::Instant};
use tomas_commander::files::Scope;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--file-agent-worker") {
        if tomas_commander::file_agent::worker().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--file-agent-run") {
        use std::io::Read;
        let result = (|| -> Result<(), String> {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(tomas_commander::file_agent::MAX_FRAME as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Read file-agent CLI request failed.")?;
            if bytes.len() > tomas_commander::file_agent::MAX_FRAME {
                return Err("File-agent CLI frame too large.".into());
            }
            let request =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid file-agent CLI request.")?;
            let result = tomas_commander::file_agent::run(
                request,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                |_, _| {},
            );
            println!(
                "{}",
                serde_json::to_string(&result)
                    .map_err(|_| "Encode file-agent CLI result failed.")?
            );
            result.map(|_| ())
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--file-agent-configure-favorites") {
        if let Err(error) = tomas_commander::file_agent::configure_from_favorites() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--answer-worker") {
        if tomas_commander::answers::worker().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--ai-worker") {
        if tomas_commander::ai::worker().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some(
            "--ai-check"
                | "--ai-smoke"
                | "--ai-check-session"
                | "--ai-grant-cmd-01"
                | "--ai-grant-answer-01"
        )
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
        if std::env::args().nth(1).as_deref() == Some("--ai-grant-answer-01") {
            ai::grant_answer_pilot(&config, &config_path)?;
            println!(
                "Owner-approved ANSWER grant persisted: original nine attempts retained; two exact public questions, official passages and Sol only, ceiling eleven. No source/model request."
            );
            return Ok(());
        }
        if std::env::args().nth(1).as_deref() == Some("--ai-grant-cmd-01") {
            ai::grant_command_pilot(&config, &config_path)?;
            println!(
                "Explicit CMD-01 grant persisted: original 5 attempts retained; four frozen CommandIntent prompts only, cumulative ceiling 9. No inference."
            );
            return Ok(());
        }
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
                    task: ai::Task::Text,
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
    #[cfg(feature = "native-desktop-tests")]
    let mut desktop_metrics = None;
    #[cfg(feature = "native-search-tests")]
    let mut search_ui_test = false;
    #[cfg(feature = "native-answer-tests")]
    let mut answer_ui_test = false;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            #[cfg(feature = "native-desktop-tests")]
            Some("--desktop-metrics") => {
                desktop_metrics = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or("--desktop-metrics requires a new output path")?,
                ));
            }
            #[cfg(feature = "native-search-tests")]
            Some("--search-ui-test") => search_ui_test = true,
            #[cfg(feature = "native-answer-tests")]
            Some("--answer-ui-test") => answer_ui_test = true,
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
    #[cfg(feature = "native-search-tests")]
    if search_ui_test && (fixture.is_none() || settings.is_none()) {
        return Err("Native search tests require explicit --fixture-root and --settings.".into());
    }
    #[cfg(feature = "native-answer-tests")]
    if answer_ui_test && (fixture.is_none() || settings.is_none()) {
        return Err("Native answer tests require explicit --fixture-root and --settings.".into());
    }
    #[cfg(feature = "native-desktop-tests")]
    if desktop_metrics.is_some() && (fixture.is_none() || settings.is_none()) {
        return Err("Desktop diagnostics require explicit --fixture-root and --settings.".into());
    }
    #[cfg(feature = "native-desktop-tests")]
    let desktop_metrics = desktop_metrics
        .map(|path| desktop_metrics::Metrics::new(&path, started))
        .transpose()?;
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
            let ledger = app::Ledger::new(creation, initial, scope, settings, started);
            #[cfg(feature = "native-desktop-tests")]
            let ledger = {
                let mut ledger = ledger;
                if let Some(metrics) = desktop_metrics {
                    ledger.enable_desktop_test(metrics);
                }
                ledger
            };
            #[cfg(feature = "native-search-tests")]
            let ledger = {
                let mut ledger = ledger;
                if search_ui_test {
                    ledger.enable_search_ui_test();
                }
                ledger
            };
            #[cfg(feature = "native-answer-tests")]
            let ledger = {
                let mut ledger = ledger;
                if answer_ui_test {
                    ledger.enable_answer_ui_test();
                }
                ledger
            };
            Ok(Box::new(ledger))
        }),
    )?;
    Ok(())
}
