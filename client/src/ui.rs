//! Окно на egui. Перерисовка только по событиям, а пока идёт звонок — ~10 раз в секунду
//! ради индикаторов; свёрнутое окно не рисуется вовсе.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use crate::audio::{self, DeviceLists};
use crate::config::Config;
use crate::net::Status;
use crate::session::Session;

const GREEN: Color32 = Color32::from_rgb(80, 200, 120);
const GRAY: Color32 = Color32::from_rgb(110, 110, 110);
const YELLOW: Color32 = Color32::from_rgb(230, 190, 60);
const RED: Color32 = Color32::from_rgb(230, 80, 80);

pub struct VoiceApp {
    cfg: Config,
    session: Option<Session>,
    error: Option<String>,
    devices: DeviceLists,
    show_settings: bool,
}

impl VoiceApp {
    pub fn new() -> VoiceApp {
        VoiceApp {
            cfg: Config::load(),
            session: None,
            error: None,
            devices: audio::list_devices(),
            show_settings: false,
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.error = None;
        let _ = self.cfg.save();
        let ctx = ctx.clone();
        match Session::connect(&self.cfg, Arc::new(move || ctx.request_repaint())) {
            Ok(s) => self.session = Some(s),
            Err(e) => self.error = Some(e),
        }
    }

    fn disconnect(&mut self) {
        if let Some(s) = self.session.take() {
            // Громкости, выставленные во время звонка, сохраняем в конфиг.
            self.cfg.volumes = s.shared.state.lock().unwrap().volumes.clone();
        }
        let _ = self.cfg.save();
    }

    fn login_form(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("login").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Сервер");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.server).hint_text("host:порт"));
            ui.end_row();
            ui.label("Ник");
            ui.text_edit_singleline(&mut self.cfg.nick);
            ui.end_row();
            ui.label("Пароль комнаты");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.passphrase).password(true));
            ui.end_row();
        });
        ui.add_space(6.0);
        if ui.button(RichText::new("Подключиться").strong()).clicked() {
            let ctx = ui.ctx().clone();
            self.connect(&ctx);
        }
    }

    fn call_view(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else { return };
        let shared = session.shared.clone();
        let now = Instant::now();
        let mut disconnect = false;

        {
            let st = shared.state.lock().unwrap();
            ui.horizontal(|ui| {
                let (txt, color) = match st.status {
                    Status::Connected => ("● В сети", GREEN),
                    Status::Connecting => ("● Подключение…", YELLOW),
                    Status::Reconnecting => ("● Переподключение…", YELLOW),
                };
                ui.colored_label(color, txt);
                if let Some(rtt) = st.rtt_ms {
                    let c = if rtt < 80.0 { GREEN } else if rtt < 150.0 { YELLOW } else { RED };
                    ui.colored_label(c, format!("пинг {rtt:.0} мс"));
                }
                if st.loss_pct > 0.0 {
                    ui.colored_label(if st.loss_pct < 5.0 { YELLOW } else { RED }, format!("потери {:.0}%", st.loss_pct));
                }
            });
            if let Some(e) = &st.audio_error {
                ui.colored_label(RED, format!("звук: {e}"));
            }
        }
        ui.separator();

        // Свой ряд: индикатор передачи и уровень микрофона.
        let (level, transmitting, my_session) = {
            let st = shared.state.lock().unwrap();
            (st.mic_level, st.transmitting, st.my_session)
        };
        ui.horizontal(|ui| {
            ui.colored_label(if transmitting { GREEN } else { GRAY }, "●");
            ui.label(RichText::new(format!("{} (ты)", self.cfg.nick)).strong());
            let db = 20.0 * level.max(1e-5).log10();
            let fill = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            ui.add(egui::ProgressBar::new(fill).desired_width(120.0).desired_height(8.0));
        });

        // Остальные участники с громкостью.
        let peers: Vec<(u32, String, bool)> = {
            let st = shared.state.lock().unwrap();
            st.roster
                .iter()
                .filter(|e| e.session_id != my_session)
                .map(|e| (e.session_id, e.nick.clone(), st.is_speaking(e.session_id, now)))
                .collect()
        };
        if peers.is_empty() {
            ui.label(RichText::new("Пока никого нет").italics().color(GRAY));
        }
        for (_, nick, speaking) in &peers {
            ui.horizontal(|ui| {
                ui.colored_label(if *speaking { GREEN } else { GRAY }, "●");
                ui.label(nick);
                let mut st = shared.state.lock().unwrap();
                let mut vol = st.volumes.get(nick).copied().unwrap_or(1.0) * 100.0;
                let r = ui.add(egui::Slider::new(&mut vol, 0.0..=200.0).suffix("%").step_by(5.0));
                if r.changed() {
                    st.volumes.insert(nick.clone(), vol / 100.0);
                }
            });
        }

        ui.separator();
        ui.horizontal(|ui| {
            let c = &shared.controls;
            let mut mic_muted = c.mic_muted.load(Ordering::Relaxed);
            let mut deaf = c.deafened.load(Ordering::Relaxed);
            let mic_label = if mic_muted { "Микрофон выкл" } else { "Микрофон вкл" };
            if ui.toggle_value(&mut mic_muted, mic_label).changed() {
                c.mic_muted.store(mic_muted, Ordering::Relaxed);
            }
            let deaf_label = if deaf { "Звук выкл" } else { "Звук вкл" };
            if ui.toggle_value(&mut deaf, deaf_label).changed() {
                c.deafened.store(deaf, Ordering::Relaxed);
            }
            if ui.button("Отключиться").clicked() {
                disconnect = true;
            }
        });

        if disconnect {
            self.disconnect();
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        let in_call = self.session.is_some();
        ui.add_enabled_ui(!in_call, |ui| {
            device_combo(ui, "Микрофон", &mut self.cfg.input_device, &self.devices.inputs);
            device_combo(ui, "Вывод", &mut self.cfg.output_device, &self.devices.outputs);
            if ui.small_button("Обновить список устройств").clicked() {
                self.devices = audio::list_devices();
            }
        });
        if in_call {
            ui.label(RichText::new("Устройства меняются после отключения").small().color(GRAY));
        }
        ui.horizontal(|ui| {
            ui.label("Чувствительность VAD");
            // Слева — чувствительнее (ниже порог).
            let r = ui.add(egui::Slider::new(&mut self.cfg.vad_threshold, 0.2..=0.95).fixed_decimals(2));
            if r.changed() {
                if let Some(s) = &self.session {
                    s.shared.controls.set_vad_threshold(self.cfg.vad_threshold);
                }
            }
            if r.drag_stopped() || r.lost_focus() {
                let _ = self.cfg.save();
            }
        });
    }
}

fn device_combo(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, list: &[String]) {
    let shown = value.clone().unwrap_or_else(|| "По умолчанию".into());
    egui::ComboBox::from_label(label).selected_text(shown).width(260.0).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "По умолчанию");
        for name in list {
            ui.selectable_value(value, Some(name.clone()), name);
        }
    });
}

impl eframe::App for VoiceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("voiceOn");
            ui.add_space(4.0);
            if self.session.is_some() {
                self.call_view(ui);
            } else {
                self.login_form(ui);
            }
            if let Some(e) = &self.error {
                ui.colored_label(RED, e);
            }
            ui.add_space(8.0);
            ui.toggle_value(&mut self.show_settings, "Настройки");
            if self.show_settings {
                self.settings(ui);
            }
        });
    }
}

impl Drop for VoiceApp {
    fn drop(&mut self) {
        self.disconnect();
    }
}
