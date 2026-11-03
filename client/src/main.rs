// В релизной сборке под Windows не открываем консольное окно.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod bot;
mod capture;
mod config;
mod dsp;
mod jitter;
mod log;
mod mixer;
mod net;
mod session;
mod ui;
mod update;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--bot") {
        if let Err(e) = bot::run(&args) {
            eprintln!("бот: {e}");
            std::process::exit(1);
        }
        return;
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("voiceOn")
            .with_app_id("voiceon")
            .with_inner_size([380.0, 360.0])
            .with_min_inner_size([300.0, 220.0]),
        ..Default::default()
    };
    if let Err(e) = eframe::run_native("voiceOn", options, Box::new(|_cc| Ok(Box::new(ui::VoiceApp::new())))) {
        eprintln!("не удалось открыть окно: {e}");
        std::process::exit(1);
    }
}
