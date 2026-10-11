use eframe::egui;
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    time::Instant,
};

#[derive(Serialize)]
struct Sample {
    controller_and_widgets_us: u128,
    pending_listing_or_aggregate: bool,
    pending_search: bool,
}

pub struct Metrics {
    file: File,
    started: Instant,
    frame: Option<Instant>,
    usable_us: Option<u128>,
    samples: Vec<Sample>,
    truncated: bool,
}

impl Metrics {
    pub fn new(path: &Path, started: Instant) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("Create new desktop diagnostic output: {error}"))?;
        Ok(Self {
            file,
            started,
            frame: None,
            usable_us: None,
            samples: Vec::new(),
            truncated: false,
        })
    }

    pub fn begin(&mut self, ctx: &egui::Context) {
        self.frame = ctx
            .input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::AccessKitActionRequest(_)
                    )
                })
            })
            .then(Instant::now);
    }

    pub fn finish(&mut self, listing_ready: bool, pending_scan: bool, pending_search: bool) {
        if listing_ready && self.usable_us.is_none() {
            self.usable_us = Some(self.started.elapsed().as_micros());
        }
        if let Some(frame) = self.frame.take() {
            if self.samples.len() < 512 {
                self.samples.push(Sample {
                    controller_and_widgets_us: frame.elapsed().as_micros(),
                    pending_listing_or_aggregate: pending_scan,
                    pending_search,
                });
            } else {
                self.truncated = true;
            }
        }
    }

    fn save(&mut self) -> Result<(), String> {
        let report = serde_json::json!({
            "test_only": true,
            "method": "Monotonic run-entry to end of first UI callback with both listings ready; input-frame logic entry to end of widget construction. Excludes OS/input queues, egui pre-logic processing, tessellation, GPU presentation and physical input-to-display latency. No input text, paths or credentials captured.",
            "startup_to_usable_widgets_us": self.usable_us,
            "input_frames": self.samples,
            "truncated": self.truncated
        });
        let bytes = serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("Encode desktop diagnostic output: {error}"))?;
        self.file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_all())
            .map_err(|error| format!("Save desktop diagnostic output: {error}"))
    }
}

impl Drop for Metrics {
    fn drop(&mut self) {
        if let Err(error) = self.save() {
            tomas_commander::platform::show_error(&error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_are_bounded_and_do_not_call_cpu_time_display_latency() {
        let path = std::env::temp_dir().join(format!(
            "tomas-desktop-metrics-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut metrics = Metrics::new(&path, Instant::now()).unwrap();
        assert!(Metrics::new(&path, Instant::now()).is_err());
        for _ in 0..513 {
            metrics.frame = Some(Instant::now());
            metrics.finish(true, true, false);
        }
        drop(metrics);
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(report["input_frames"].as_array().unwrap().len(), 512);
        assert_eq!(report["truncated"], true);
        assert!(report["startup_to_usable_widgets_us"].is_number());
        assert!(report["method"].as_str().unwrap().contains("Excludes"));
    }
}
