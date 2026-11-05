//! Окно на egui. Перерисовка только по событиям, а пока идёт звонок — ~10 раз в секунду
//! ради индикаторов; свёрнутое окно не рисуется вовсе.

pub(crate) mod theme;
pub(crate) mod widgets;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, CornerRadius, Frame, Layout, Margin, RichText, Stroke, TextWrapMode, Ui, vec2};
use egui_phosphor::regular as ph;

use crate::audio::{self, DeviceLists};
use crate::config::Config;
use crate::modules::Module;
use crate::net::Status;
use crate::session::Session;
use crate::soundpad::Soundpad;

use theme::{Palette, pal, semibold};

/// Сколько показывать последнюю ошибку звука.
const AUDIO_ERROR_SHOW: Duration = Duration::from_secs(5);
const VAD_RANGE: (f32, f32) = (0.2, 0.95);

#[derive(PartialEq)]
enum Screen {
    Main,
    Settings,
}

/// Если панель с треем пропала, а окно спрятано, через столько показываем окно.
const TRAY_LOST_SHOW: Duration = Duration::from_secs(10);
const RESTORE_POS_DELAY: Duration = Duration::from_millis(150);

/// Как запущено приложение.
pub struct Launch {
    /// `--tray`: стартовать спрятанным в трей.
    pub start_hidden: bool,
    /// Окно можно спрятать целиком; иначе (чистый Wayland) только свернуть.
    pub can_hide: bool,
    /// Самоустановка не удалась — показываем заметку.
    pub install_error: Option<String>,
}

pub struct VoiceApp {
    cfg: Config,
    session: Option<Session>,
    error: Option<String>,
    devices: DeviceLists,
    screen: Screen,
    /// Затухающий пик вероятности речи на полосе VAD.
    vad_peak: f32,
    update: crate::update::Shared,
    tray: Option<crate::tray::Tray>,
    /// Панель с треем на месте (Linux: StatusNotifierWatcher); с какого момента её нет.
    tray_lost: Option<Instant>,
    can_hide: bool,
    /// Окно спрятано в трей (или свёрнуто, если спрятать нельзя).
    hidden: bool,
    /// Где было окно, когда его спрятали; когда вернуть его туда после показа.
    hidden_pos: Option<egui::Pos2>,
    /// (куда, когда, уже ставили и осталось проверить).
    restore_pos: Option<(egui::Pos2, Instant, bool)>,
    /// Поправка к позиции: KWin под X11 считает запрошенную позицию позицией рамки,
    /// а winit сообщает позицию содержимого — без поправки окно сползает на высоту заголовка.
    pos_fix: egui::Vec2,
    /// Спрятать окно на первом кадре (`--tray`).
    hide_on_start: bool,
    /// Окно свернули в этом проходе (только без `can_hide`).
    hid_this_pass: bool,
    /// «Выход» из трея: закрытие окна больше не прячет его.
    quitting: bool,
    /// Ошибка автозапуска для экрана настроек.
    autostart_error: Option<String>,
    /// Модуль саундпада: слоты, дровер, диалог обрезки.
    soundpad: Soundpad,
}

impl VoiceApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> VoiceApp {
        theme::install(&cc.egui_ctx);
        crate::single_instance::serve(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let update = crate::update::spawn(move || ctx.request_repaint());
        let tray = match crate::tray::Tray::spawn(&cc.egui_ctx) {
            Ok(t) => Some(t),
            Err(e) => {
                crate::log::info(&format!("трей недоступен: {e}"));
                None
            }
        };
        let cfg = Config::load();
        let soundpad = crate::soundpad::import::sounds_dir()
            .map(|d| Soundpad::load(&cfg, &d))
            .unwrap_or_default();
        // Путь к exe мог поменяться (установка) — прописываем автозапуск заново.
        let autostart_error = if cfg.autostart { crate::autostart::set(true).err() } else { None };
        if let Some(e) = &autostart_error {
            crate::log::info(&format!("автозапуск: {e}"));
        }
        VoiceApp {
            cfg,
            session: None,
            error: launch.install_error.map(|e| format!("Не удалось установить приложение: {e}")),
            devices: audio::list_devices(),
            screen: Screen::Main,
            vad_peak: 0.0,
            update,
            // Без трея спрятанное окно не вернуть — тогда стартуем как обычно.
            hide_on_start: launch.start_hidden && tray.is_some(),
            tray,
            tray_lost: None,
            can_hide: launch.can_hide,
            hidden: false,
            hidden_pos: None,
            restore_pos: None,
            pos_fix: egui::Vec2::ZERO,
            hid_this_pass: false,
            quitting: false,
            autostart_error,
            soundpad,
        }
    }

    /// Трей есть и виден — значит, окно можно прятать.
    fn tray_usable(&self) -> bool {
        self.tray.is_some() && self.tray_lost.is_none()
    }

    fn hide(&mut self, ctx: &egui::Context) {
        if self.can_hide {
            // Оконный менеджер заново расставляет показанное окно — запомним, где оно было.
            self.hidden_pos = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.hid_this_pass = true;
        }
        self.hidden = true;
    }

    fn show(&mut self, ctx: &egui::Context) {
        // Место возвращаем чуть позже: сразу после показа оконный менеджер ещё расставляет окно.
        if self.hidden {
            self.restore_pos = self.hidden_pos.take().map(|p| (p, Instant::now() + RESTORE_POS_DELAY, false));
            ctx.request_repaint_after(RESTORE_POS_DELAY);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        if self.can_hide {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.hidden = false;
        ctx.request_repaint();
    }

    /// Возвращает показанное окно на прежнее место; один раз проверяет, куда оно встало,
    /// и запоминает поправку на будущее.
    fn restore_position(&mut self, ctx: &egui::Context) {
        let Some((target, at, placed)) = self.restore_pos else { return };
        let left = at.saturating_duration_since(Instant::now());
        if !left.is_zero() {
            ctx.request_repaint_after(left);
            return;
        }
        if !placed {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target - self.pos_fix));
            self.restore_pos = Some((target, Instant::now() + RESTORE_POS_DELAY, true));
            ctx.request_repaint_after(RESTORE_POS_DELAY);
            return;
        }
        self.restore_pos = None;
        let Some(now) = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min) else { return };
        let off = now - target;
        // Сдвиг на величину рамки (не больше ~100 px) поправляем; большой — пользователь двигал окно.
        if off != egui::Vec2::ZERO && off.x.abs() < 100.0 && off.y.abs() < 100.0 {
            self.pos_fix += off;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target - self.pos_fix));
        }
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        use crate::tray::Event;
        while let Some(ev) = self.tray.as_ref().and_then(|t| t.try_recv()) {
            match ev {
                Event::Show => self.show(ctx),
                Event::ToggleMic | Event::ToggleSound => {
                    if let Some(s) = &self.session {
                        let c = &s.shared.controls;
                        let flag = if ev == Event::ToggleMic { &c.mic_muted } else { &c.deafened };
                        flag.fetch_xor(true, Ordering::Relaxed);
                    }
                }
                Event::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    ctx.request_repaint(); // при спрятанном окне закрытие придёт следующим проходом
                }
                Event::HostOffline => {
                    self.tray_lost.get_or_insert_with(Instant::now);
                }
                Event::HostOnline => self.tray_lost = None,
            }
        }
        // Панель с треем пропала надолго — спрятанное окно больше нечем вернуть.
        if let Some(t) = self.tray_lost
            && self.hidden
        {
            let left = TRAY_LOST_SHOW.saturating_sub(t.elapsed());
            if left.is_zero() {
                self.show(ctx);
            } else {
                ctx.request_repaint_after(left);
            }
        }
        if let Some(tray) = &mut self.tray {
            let state = match &self.session {
                Some(s) => crate::tray::State {
                    in_call: true,
                    mic_muted: s.shared.controls.mic_muted.load(Ordering::Relaxed),
                    deafened: s.shared.controls.deafened.load(Ordering::Relaxed),
                },
                None => crate::tray::State::default(),
            };
            tray.set_state(state);
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        self.error = None;
        let _ = self.cfg.save();
        self.soundpad.stop_all(None); // локальное предпрослушивание — в звонок не тащим
        let ctx = ctx.clone();
        match Session::connect(&self.cfg, Arc::new(move || ctx.request_repaint())) {
            Ok(s) => self.session = Some(s),
            Err(e) => self.error = Some(e),
        }
    }

    fn disconnect(&mut self) {
        self.soundpad.stop_all(self.session.as_ref().map(|s| s.shared.as_ref()));
        if let Some(s) = self.session.take() {
            // Громкости, выставленные во время звонка, сохраняем в конфиг.
            self.cfg.volumes = s.shared.state.lock().unwrap().volumes.clone();
        }
        let _ = self.cfg.save();
    }

    // ---------- вход ----------

    fn login_screen(&mut self, ui: &mut Ui) {
        let p = pal(ui);
        egui::Panel::bottom("login_footer").show_separator_line(false).frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(0, 10))).show(ui, |ui| {
            ui.vertical_centered(|ui| version_label(ui, &p, "v", &self.update));
        });
        if Soundpad::enabled(&self.cfg) && self.soundpad.drawer_open {
            egui::Panel::bottom("soundpad_drawer")
                .show_separator_line(true)
                .frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, 10)))
                .show(ui, |ui| {
                    crate::soundpad::ui::drawer(ui, &mut self.soundpad, None, &mut self.cfg, &p);
                });
        }
        egui::CentralPanel::default().frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(24, 16))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 16.0;
                ui.horizontal(|ui| {
                    widgets::logo(ui, 28.0, p.accent);
                    ui.label(RichText::new("voiceOn").font(semibold(20.0)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, ph::SLIDERS_HORIZONTAL, 36.0, p.muted, "Настройки").clicked() {
                            self.screen = Screen::Settings;
                        }
                        if Soundpad::enabled(&self.cfg) {
                            let color = if self.soundpad.drawer_open { p.accent } else { p.muted };
                            if widgets::icon_button(ui, ph::MUSIC_NOTES, 36.0, color, "Саундпад").clicked() {
                                self.soundpad.drawer_open = !self.soundpad.drawer_open;
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                labelled(ui, &p, "Сервер", |ui| {
                    ui.add(input(&mut self.cfg.server).hint_text("host:порт"));
                });
                labelled(ui, &p, "Ник", |ui| {
                    ui.add(input(&mut self.cfg.nick));
                });
                labelled(ui, &p, "Пароль комнаты", |ui| {
                    ui.add(input(&mut self.cfg.passphrase).password(true));
                });
                if widgets::primary_button(ui, ph::MICROPHONE, "Подключиться", &p).clicked() {
                    let ctx = ui.ctx().clone();
                    self.connect(&ctx);
                }
                if let Some(e) = self.error.clone() {
                    error_box(ui, &p, &e);
                }
            });
        });
    }

    // ---------- звонок ----------

    fn call_screen(&mut self, ui: &mut Ui) {
        let Some(session) = &self.session else { return };
        let shared = session.shared.clone();
        let p = pal(ui);
        let now = Instant::now();
        let mut disconnect = false;
        let mut open_settings = false;

        // Снимок состояния под одним замком.
        let (status, rtt, loss, my_session, level, transmitting, audio_error, peers) = {
            let st = shared.state.lock().unwrap();
            let peers: Vec<(String, bool, f32)> = st
                .roster
                .iter()
                .filter(|e| e.session_id != st.my_session)
                .map(|e| (e.nick.clone(), st.is_speaking(e.session_id, now), st.volumes.get(&e.nick).copied().unwrap_or(1.0)))
                .collect();
            let err = st.audio_error.as_ref().filter(|(_, t)| t.elapsed() < AUDIO_ERROR_SHOW).map(|(m, _)| m.clone());
            (st.status, st.rtt_ms, st.loss_pct, st.my_session, st.mic_level, st.transmitting, err, peers)
        };
        let _ = my_session;
        let total = peers.len() + 1;

        egui::Panel::top("call_header")
            .show_separator_line(true)
            .frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::logo(ui, 22.0, p.accent);
                    ui.label(RichText::new("voiceOn").font(semibold(16.0)));
                    ping_pill(ui, &p, status, rtt, loss);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, ph::SLIDERS_HORIZONTAL, 36.0, p.muted, "Настройки").clicked() {
                            open_settings = true;
                        }
                    });
                });
            });

        let tall = ui.ctx().content_rect().height() >= 400.0;
        let pad = if tall { 16 } else { 8 };
        let pad_on = Soundpad::enabled(&self.cfg);
        let mut toggle_drawer = false;
        egui::Panel::bottom("call_footer")
            .show_separator_line(true)
            .frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, pad)))
            .show(ui, |ui| {
                let c = &shared.controls;
                let mut mic_muted = c.mic_muted.load(Ordering::Relaxed);
                let mut deaf = c.deafened.load(Ordering::Relaxed);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let buttons = if pad_on { 4.0 } else { 3.0 };
                    ui.add_space(((ui.available_width() - 52.0 * buttons - 16.0 * (buttons - 1.0)) / 2.0).max(0.0));
                    let (fill, fg) = if mic_muted { (p.round_btn_muted, p.danger) } else { (p.round_btn, p.text) };
                    let (icon, tip) = if mic_muted { (ph::MICROPHONE_SLASH, "Включить микрофон") } else { (ph::MICROPHONE, "Выключить микрофон") };
                    if widgets::round_button(ui, icon, fill, fg, tip).clicked() {
                        mic_muted = !mic_muted;
                        c.mic_muted.store(mic_muted, Ordering::Relaxed);
                    }
                    let (fill, fg) = if deaf { (p.round_btn_muted, p.danger) } else { (p.round_btn, p.text) };
                    let (icon, tip) = if deaf { (ph::SPEAKER_SLASH, "Включить звук") } else { (ph::SPEAKER_HIGH, "Выключить звук") };
                    if widgets::round_button(ui, icon, fill, fg, tip).clicked() {
                        deaf = !deaf;
                        c.deafened.store(deaf, Ordering::Relaxed);
                    }
                    if pad_on {
                        let (fill, fg) = if self.soundpad.drawer_open { (p.accent, p.on_accent) } else { (p.round_btn, p.text) };
                        if widgets::round_button(ui, ph::MUSIC_NOTES, fill, fg, "Саундпад").clicked() {
                            toggle_drawer = true;
                        }
                    }
                    if widgets::round_button(ui, ph::PHONE_DISCONNECT, p.danger, egui::Color32::WHITE, "Отключиться").clicked() {
                        disconnect = true;
                    }
                });
            });

        if Soundpad::enabled(&self.cfg) && self.soundpad.drawer_open {
            egui::Panel::bottom("soundpad_drawer")
                .show_separator_line(true)
                .frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, 10)))
                .show(ui, |ui| {
                    crate::soundpad::ui::drawer(ui, &mut self.soundpad, Some(session), &mut self.cfg, &p);
                });
        }

        let mic_muted = shared.controls.mic_muted.load(Ordering::Relaxed);
        let mut new_vols: Vec<(String, f32)> = Vec::new();
        egui::CentralPanel::default().frame(Frame::NONE.fill(p.bg)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(8.0);
                if let Some(e) = &audio_error {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.scope(|ui| {
                            ui.set_max_width(ui.available_width() - 8.0);
                            error_box(ui, &p, &format!("Звук: {e}"));
                        });
                    });
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("В КОМНАТЕ · {total}")).size(12.0).font(semibold(12.0)).color(p.muted),
                    );
                });
                ui.add_space(2.0);

                // Свой ряд.
                row_frame(ui, Some(p.surface), |ui| {
                    widgets::avatar(ui, &self.cfg.nick, transmitting, &p);
                    let icon_w = 22.0;
                    let w = (ui.available_width() - icon_w - ui.spacing().item_spacing.x).max(40.0);
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let mut name = egui::text::LayoutJob::default();
                        name.wrap.max_rows = 1;
                        name.append(&self.cfg.nick, 0.0, egui::TextFormat::simple(semibold(14.0), p.text));
                        name.append(" (ты)", 0.0, egui::TextFormat::simple(egui::FontId::proportional(14.0), p.muted));
                        ui.add(egui::Label::new(name).truncate());
                        let db = 20.0 * level.max(1e-5).log10();
                        widgets::level_bar(ui, (db + 60.0) / 60.0, &p);
                    });
                    let (icon, col) = if mic_muted {
                        (ph::MICROPHONE_SLASH, p.danger)
                    } else if transmitting {
                        (ph::MICROPHONE, p.accent)
                    } else {
                        (ph::MICROPHONE, p.muted)
                    };
                    ui.label(RichText::new(icon).font(theme::icon_font(18.0)).color(col));
                });

                if peers.is_empty() {
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Пока никого нет").color(p.dim)));
                }
                for (nick, speaking, vol) in &peers {
                    row_frame(ui, None, |ui| {
                        widgets::avatar(ui, nick, *speaking, &p);
                        let w = ui.available_width();
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.add(egui::Label::new(RichText::new(nick).font(semibold(14.0)).color(p.text)).truncate());
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let icon = if *vol <= 0.0 { ph::SPEAKER_SLASH } else { ph::SPEAKER_HIGH };
                                ui.label(RichText::new(icon).font(theme::icon_font(14.0)).color(p.muted));
                                let mut v = *vol;
                                let sw = (ui.available_width() - 40.0 - 8.0).max(30.0);
                                if widgets::volume_slider(ui, sw, &mut v, &p) {
                                    new_vols.push((nick.clone(), v));
                                }
                                let shown = new_vols.iter().rev().find(|(n, _)| n == nick).map(|(_, v)| *v).unwrap_or(*vol);
                                ui.allocate_ui_with_layout(vec2(40.0, 16.0), Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{:.0}%", shown * 100.0)).size(12.0).color(p.muted));
                                });
                            });
                        });
                    });
                }
            });
        });

        if !new_vols.is_empty() {
            let mut st = shared.state.lock().unwrap();
            for (n, v) in new_vols {
                st.volumes.insert(n, v);
            }
        }
        if open_settings {
            self.screen = Screen::Settings;
        }
        if toggle_drawer {
            self.soundpad.drawer_open = !self.soundpad.drawer_open;
        }
        if disconnect {
            self.disconnect();
        }
    }

    // ---------- настройки ----------

    fn settings_screen(&mut self, ui: &mut Ui) {
        let p = pal(ui);
        let in_call = self.session.is_some();
        let mut back = false;
        egui::Panel::top("settings_header")
            .show_separator_line(true)
            .frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(12, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if widgets::icon_button(ui, ph::ARROW_LEFT, 40.0, p.text, "Назад").clicked() {
                        back = true;
                    }
                    ui.label(RichText::new("Настройки").font(semibold(16.0)));
                });
            });
        egui::Panel::bottom("settings_footer").show_separator_line(false).frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(0, 10))).show(ui, |ui| {
            ui.vertical_centered(|ui| version_label(ui, &p, "voiceOn v", &self.update));
        });

        // Живая вероятность речи для полосы VAD; пик затухает.
        let prob = self.session.as_ref().map(|s| s.shared.state.lock().unwrap().voice_prob).unwrap_or(0.0);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        self.vad_peak = prob.max(self.vad_peak - dt * 0.6);

        egui::CentralPanel::default().frame(Frame::NONE.fill(p.bg).inner_margin(Margin::symmetric(16, 16))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 20.0;
                ui.add_enabled_ui(!in_call, |ui| {
                    labelled(ui, &p, "Микрофон", |ui| {
                        device_combo(ui, &p, "dev_in", &mut self.cfg.input_device, &self.devices.inputs);
                    });
                    labelled(ui, &p, "Вывод звука", |ui| {
                        device_combo(ui, &p, "dev_out", &mut self.cfg.output_device, &self.devices.outputs);
                    });
                });
                if in_call {
                    ui.label(RichText::new("Устройства меняются после отключения").size(12.0).color(p.dim));
                } else if ui.add(egui::Button::new(RichText::new("Обновить список устройств").size(12.0).color(p.muted)).frame(false)).clicked() {
                    self.devices = audio::list_devices();
                }

                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Голосовая активация").size(13.0).color(p.muted));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("порог {:.2}", self.cfg.vad_threshold)).size(12.0).color(p.dim));
                        });
                    });
                    let r = widgets::vad_bar(ui, prob, self.vad_peak, &mut self.cfg.vad_threshold, VAD_RANGE, &p);
                    if r.changed() {
                        if let Some(s) = &self.session {
                            s.shared.controls.set_vad_threshold(self.cfg.vad_threshold);
                        }
                    }
                    if r.drag_stopped() || r.clicked() {
                        let _ = self.cfg.save();
                    }
                    let hint = if in_call {
                        "Двигай белую черту: всё, что громче неё, уходит в эфир. Скажи что-нибудь, чтобы проверить."
                    } else {
                        "Двигай белую черту: всё, что громче неё, уходит в эфир. Уровень голоса виден во время звонка."
                    };
                    ui.label(RichText::new(hint).size(12.0).color(p.dim));
                });

                Frame::NONE.fill(p.surface).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Шумодав").size(14.0).color(p.text));
                            ui.label(RichText::new("убирает клавиатуру и вентиляторы").size(12.0).color(p.dim));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut on = self.cfg.denoise;
                            if widgets::toggle(ui, &mut on, &p) {
                                self.cfg.denoise = on;
                                if let Some(s) = &self.session {
                                    s.shared.controls.denoise.store(on, Ordering::Relaxed);
                                }
                                let _ = self.cfg.save();
                            }
                        });
                    });
                });

                Frame::NONE.fill(p.surface).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Саундпад").size(14.0).color(p.text));
                            ui.label(RichText::new("звуки по кнопке в звонке").size(12.0).color(p.dim));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut on = Soundpad::enabled(&self.cfg);
                            if widgets::toggle(ui, &mut on, &p) {
                                Soundpad::set_enabled(&mut self.cfg, on);
                                crate::log::info(&format!("модуль {}: {on}", self.soundpad.id()));
                                if !on {
                                    self.soundpad.drawer_open = false;
                                }
                                let _ = self.cfg.save();
                            }
                        });
                    });
                });

                Frame::NONE.fill(p.surface).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Запускать с системой").size(14.0).color(p.text));
                            ui.label(RichText::new("сразу в трей, без окна").size(12.0).color(p.dim));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut on = self.cfg.autostart;
                            if widgets::toggle(ui, &mut on, &p) {
                                match crate::autostart::set(on) {
                                    Ok(()) => {
                                        self.cfg.autostart = on;
                                        self.autostart_error = None;
                                        let _ = self.cfg.save();
                                    }
                                    Err(e) => {
                                        crate::log::info(&format!("автозапуск: {e}"));
                                        self.autostart_error = Some(e);
                                    }
                                }
                            }
                        });
                    });
                });
                if let Some(e) = self.autostart_error.clone() {
                    error_box(ui, &p, &format!("Автозапуск: {e}"));
                }
            });
        });
        if back {
            self.screen = Screen::Main;
        }
    }
}

fn input(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .margin(Margin::symmetric(12, 12))
        .align(egui::Align2::LEFT_CENTER)
}

/// Подпись сверху и поле под ней.
fn labelled(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(label).size(13.0).color(p.muted));
        add(ui);
    });
}

fn version_label(ui: &mut Ui, p: &Palette, prefix: &str, update: &crate::update::Shared) {
    use crate::update::Status;
    let extra = match &*update.lock().unwrap() {
        Status::Downloading(v) => format!(" · загружаю {v}…"),
        Status::Installed(v) => format!(" · {v} установится после звонка"),
        _ => String::new(),
    };
    ui.label(RichText::new(format!("{prefix}{}{extra}", env!("CARGO_PKG_VERSION"))).size(12.0).color(p.dim));
}

fn error_box(ui: &mut Ui, p: &Palette, text: &str) {
    Frame::NONE.fill(p.err_bg).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.label(RichText::new(ph::WARNING_CIRCLE).font(theme::icon_font(18.0)).color(p.err_text));
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(p.err_text)).wrap());
        });
    });
}

fn row_frame(ui: &mut Ui, fill: Option<egui::Color32>, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let w = ui.available_width() - 8.0;
        let frame = Frame::NONE.corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(8));
        let frame = if let Some(f) = fill { frame.fill(f) } else { frame };
        frame.show(ui, |ui| {
            ui.set_width(w - 16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                add(ui);
            });
        });
    });
}

/// Пилюля с пингом: цвет по качеству связи; при подключении — жёлтая с текстом статуса.
fn ping_pill(ui: &mut Ui, p: &Palette, status: Status, rtt: Option<f32>, loss: f32) {
    let (text, color) = match (status, rtt) {
        (Status::Connecting, _) => ("Подключение…".to_string(), p.warn),
        (Status::Reconnecting, _) => ("Переподключение…".to_string(), p.warn),
        (Status::Connected, Some(r)) => {
            let c = if r < 80.0 { p.accent } else if r < 150.0 { p.warn } else { p.danger };
            let t = if loss > 0.0 { format!("{r:.0} мс · {loss:.0}%") } else { format!("{r:.0} мс") };
            (t, c)
        }
        (Status::Connected, None) => ("в сети".to_string(), p.accent),
    };
    let r = Frame::NONE
        .fill(color.gamma_multiply(if ui.visuals().dark_mode { 0.16 } else { 0.18 }))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.horizontal(|ui| {
                widgets::signal_icon(ui, color);
                ui.add(egui::Label::new(RichText::new(text).font(semibold(12.0)).color(color)).wrap_mode(TextWrapMode::Truncate));
            });
        });
    if loss > 0.0 {
        r.response.on_hover_text(format!("Потери пакетов: {loss:.0}%"));
    }
}

fn device_combo(ui: &mut Ui, p: &Palette, id: &str, value: &mut Option<String>, list: &[String]) {
    let shown = value.clone().unwrap_or_else(|| "По умолчанию".into());
    let width = ui.available_width();
    ui.scope(|ui| {
        let s = ui.style_mut();
        s.spacing.button_padding = vec2(12.0, 12.0);
        s.spacing.combo_width = width;
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border_input);
        egui::ComboBox::from_id_salt(id)
            .selected_text(RichText::new(shown).size(14.0).color(p.text))
            .width(width)
            .wrap_mode(TextWrapMode::Truncate)
            .show_ui(ui, |ui| {
                ui.set_min_width(width - 20.0);
                ui.set_max_width(width - 20.0);
                ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                ui.selectable_value(value, None, "По умолчанию");
                for name in list {
                    ui.selectable_value(value, Some(name.clone()), name);
                }
            });
    });
}

impl eframe::App for VoiceApp {
    /// Зовётся и при спрятанном окне: трей, закрытие в трей и перезапуск после обновления.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.hide_on_start) {
            self.hide(ctx);
            self.hidden_pos = None; // окно ещё не расставлено — при показе место выберет оконный менеджер
        }
        self.handle_tray(ctx);
        if crate::single_instance::take_show_request() {
            self.show(ctx);
        }
        self.restore_position(ctx);
        // Крестик прячет окно в трей, звонок продолжается; «Выход» в трее закрывает по-настоящему.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.tray_usable() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        // Новая версия уже на месте бинарника — перезапускаемся, когда нет звонка.
        let installed = match &*self.update.lock().unwrap() {
            crate::update::Status::Installed(tag) => Some(tag.clone()),
            _ => None,
        };
        if let (Some(tag), None) = (installed, &self.session) {
            let _ = self.cfg.save();
            self.tray = None; // убрать значок до выхода
            crate::update::restart(&tag, self.hidden);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Свёрнутое (не спрятанное) окно вернули с панели задач: ui снова рисуется.
        if !self.can_hide && !std::mem::take(&mut self.hid_this_pass) {
            self.hidden = false;
        }
        match self.screen {
            Screen::Settings => self.settings_screen(ui),
            Screen::Main if self.session.is_some() => self.call_screen(ui),
            Screen::Main => self.login_screen(ui),
        }
        // На экране входа перерисовываемся, пока играет предпрослушивание (подсветка слота).
        if self.session.is_none()
            && !self.hidden
            && self.soundpad.preview.as_ref().is_some_and(|p| p.is_playing())
        {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // Диалог обрезки саундпада поверх любого экрана.
        if self.soundpad.trim.is_some() {
            crate::soundpad::ui::trim_dialog(ui.ctx(), &mut self.soundpad, &mut self.cfg);
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        // Индикаторы живут только в звонке и в видимом окне; иначе перерисовка по событиям.
        if self.session.is_some() && !self.hidden {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl Drop for VoiceApp {
    fn drop(&mut self) {
        self.disconnect();
    }
}
