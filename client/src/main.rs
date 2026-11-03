// В релизной сборке под Windows не открываем консольное окно.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod autostart;
mod bot;
mod capture;
mod config;
mod dsp;
mod install;
mod jitter;
mod log;
mod mixer;
mod net;
mod session;
mod tray;
mod ui;
mod update;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("voiceon {}", install::version());
        return;
    }
    if args.iter().any(|a| a == "--bot") {
        if let Err(e) = bot::run(&args) {
            eprintln!("бот: {e}");
            std::process::exit(1);
        }
        return;
    }

    let install_error = match install::run(&args) {
        install::Outcome::Relaunched => return,
        install::Outcome::Continue(err) => err,
    };

    // Linux: winit под Wayland не умеет ни прятать окно, ни поднимать его обратно
    // (set_visible и focus_window там пустые), поэтому для сворачивания в трей
    // работаем через X11 (XWayland), если он есть. VOICEON_WAYLAND=1 — остаться на Wayland.
    #[cfg(target_os = "linux")]
    let use_x11 = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
        && std::env::var_os("VOICEON_WAYLAND").is_none();
    #[cfg(target_os = "linux")]
    let can_hide = use_x11 || std::env::var_os("WAYLAND_DISPLAY").is_none();
    #[cfg(not(target_os = "linux"))]
    let can_hide = true;

    let launch = ui::Launch {
        start_hidden: args.iter().any(|a| a == "--tray" || a == "--minimized"),
        can_hide,
        install_error,
    };
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("voiceOn")
            .with_app_id("voiceon")
            .with_inner_size([380.0, 560.0])
            .with_min_inner_size([300.0, 220.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("встроенная иконка")),
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    if use_x11 {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11 as _;
            b.with_x11();
        }));
    }
    if let Err(e) = eframe::run_native("voiceOn", options, Box::new(|cc| Ok(Box::new(ui::VoiceApp::new(cc, launch))))) {
        eprintln!("не удалось открыть окно: {e}");
        std::process::exit(1);
    }
}
