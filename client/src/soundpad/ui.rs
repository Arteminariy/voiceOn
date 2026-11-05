//! Дровер саундпада и вспомогательный UI (диалог обрезки — отдельным шагом).

use std::sync::atomic::Ordering;

use eframe::egui::{self, Align, Align2, CornerRadius, Layout, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as ph;

use crate::config::Config;
use crate::session::Session;
use crate::soundpad::{SLOT_COUNT, Soundpad, TrimState, import};
use crate::ui::theme::{Palette, semibold};
use crate::ui::widgets;

const SLOT_SIZE: f32 = 56.0;

/// Действие по слоту, собранное за кадр (применяется после отрисовки кнопок).
enum Act {
    Play(usize),
    Import(usize),
    Replace(usize),
    Remove(usize),
}

/// Дровер над футером. `session: None` на экране входа — звуки играют только локально.
pub fn drawer(ui: &mut Ui, pad: &mut Soundpad, session: Option<&Session>, cfg: &mut Config, p: &Palette) {
    // Доигравшее предпрослушивание убираем, чтобы подсветка слота погасла.
    if pad.preview.as_ref().is_some_and(|pr| !pr.is_playing()) {
        pad.preview = None;
        pad.preview_slot = None;
    }
    let deaf = session.map(|s| s.shared.controls.deafened.load(Ordering::Relaxed)).unwrap_or(false);

    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal(|ui| {
        ui.label(RichText::new("САУНДПАД").size(12.0).font(semibold(12.0)).color(p.muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, ph::STOP, 28.0, p.muted, "Остановить все звуки").clicked() {
                pad.stop_all(session.map(|s| s.shared.as_ref()));
            }
            let mut v = cfg.soundpad.volume;
            if widgets::volume_slider(ui, 120.0, &mut v, p) {
                cfg.soundpad.volume = v;
                if let Some(s) = session {
                    s.shared.pad.set_gain(v);
                }
                let _ = cfg.save();
            }
        });
    });

    let mut action: Option<Act> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let first_empty = pad.slots.iter().position(|s| s.is_none());
        for i in 0..SLOT_COUNT {
            if pad.slots[i].is_some() {
                let active = match session {
                    Some(s) => s.shared.pad.slot_active(i),
                    None => pad.preview_slot == Some(i) && pad.preview.is_some(),
                };
                let resp = slot_button(ui, &pad.slots[i].as_ref().unwrap().name, i, active, deaf, p);
                if resp.clicked() {
                    action = Some(Act::Play(i));
                }
                resp.context_menu(|ui| {
                    if ui.button("Заменить…").clicked() {
                        action = Some(Act::Replace(i));
                        ui.close();
                    }
                    if ui.button("Удалить").clicked() {
                        action = Some(Act::Remove(i));
                        ui.close();
                    }
                });
            } else if first_empty == Some(i) {
                if plus_button(ui, p).clicked() {
                    action = Some(Act::Import(i));
                }
            }
        }
    });

    match action {
        Some(Act::Play(i)) => match session {
            Some(s) => pad.play(&s.shared, i),
            None => pad.play_local(cfg, i),
        },
        Some(Act::Import(i)) | Some(Act::Replace(i)) => start_import(pad, i),
        Some(Act::Remove(i)) => {
            if let Some(d) = import::sounds_dir() {
                pad.remove(cfg, &d, i);
            }
        }
        None => {}
    }
}

/// Квадратная кнопка загруженного слота: иконка, номер, акцентная заливка во время игры.
fn slot_button(ui: &mut Ui, name: &str, idx: usize, active: bool, deaf: bool, p: &Palette) -> Response {
    let sense = if deaf { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(vec2(SLOT_SIZE, SLOT_SIZE), sense);
    let fill = if deaf {
        p.surface.gamma_multiply(0.6)
    } else if active {
        p.accent
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        p.surface
    };
    ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, p.border_input), egui::StrokeKind::Inside);
    let fg = if deaf {
        p.dim
    } else if active {
        p.on_accent
    } else {
        p.text
    };
    ui.painter().text(rect.center() - vec2(0.0, 7.0), Align2::CENTER_CENTER, ph::MUSIC_NOTE, crate::ui::theme::icon_font(20.0), fg);
    ui.painter().text(
        rect.center() + vec2(0.0, 12.0),
        Align2::CENTER_CENTER,
        format!("{}", idx + 1),
        egui::FontId::proportional(10.0),
        fg,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if deaf { egui::CursorIcon::NotAllowed } else { egui::CursorIcon::PointingHand });
    }
    let tip = if deaf { format!("{name}\nВключи звук, чтобы играть звуки") } else { name.to_string() };
    resp.on_hover_text(tip)
}

/// Пунктирный «плюс» на месте пустого слота.
fn plus_button(ui: &mut Ui, p: &Palette) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(SLOT_SIZE, SLOT_SIZE), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let stroke = Stroke::new(1.5, if resp.hovered() { p.muted } else { p.border_input });
    dashed_rect(ui.painter(), rect.shrink(1.0), stroke);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, ph::PLUS, crate::ui::theme::icon_font(20.0), p.muted);
    resp.on_hover_text("Добавить звук")
}

/// Пунктирная рамка (egui штриховку не умеет — рисуем сами).
fn dashed_rect(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let dash = |painter: &egui::Painter, a: Pos2, b: Pos2| {
        let v = b - a;
        let len = v.length();
        if len <= 0.0 {
            return;
        }
        let dir = v / len;
        let mut d = 0.0;
        while d < len {
            let e = (d + 4.0).min(len);
            painter.line_segment([a + dir * d, a + dir * e], stroke);
            d = e + 3.0;
        }
    };
    dash(painter, rect.left_top(), rect.right_top());
    dash(painter, rect.right_top(), rect.right_bottom());
    dash(painter, rect.right_bottom(), rect.left_bottom());
    dash(painter, rect.left_bottom(), rect.left_top());
}

/// Выбор файла и старт фонового декода; диалог обрезки подхватит `TrimState`.
fn start_import(pad: &mut Soundpad, slot: usize) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Звуки", &["mp3", "wav", "ogg", "flac"])
        .pick_file()
    else {
        return;
    };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("звук").to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = import::decode(&path).map(|source| {
            let peaks = import::peaks(&source, 512);
            (source, peaks)
        });
        let _ = tx.send(r);
    });
    pad.trim = Some(TrimState {
        source: Vec::new(),
        peaks: Vec::new(),
        name,
        start: 0.0,
        end: 0.0,
        target_slot: slot,
        busy: true,
        error: None,
        rx: Some(rx),
    });
}
