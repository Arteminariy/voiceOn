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
    let deaf = session.map(|s| s.shared.controls.deafened.load(Ordering::Relaxed)).unwrap_or(false);

    ui.spacing_mut().item_spacing.y = 8.0;
    ui.horizontal(|ui| {
        ui.label(RichText::new("САУНДПАД").size(12.0).font(semibold(12.0)).color(p.muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let stop = egui::Button::new(
                RichText::new(ph::STOP).font(crate::ui::theme::icon_font(14.0)).color(p.muted),
            );
            if ui.add(stop).on_hover_text("Остановить все звуки").clicked() {
                pad.stop_all(session.map(|s| s.shared.as_ref()));
            }
            let mut v = cfg.soundpad.volume;
            if widgets::volume_slider(ui, 120.0, &mut v, p) {
                cfg.soundpad.volume = v;
                if let Some(s) = session {
                    s.shared.pad.set_gain(v);
                }
                if let Some(l) = &pad.local {
                    l.pad.set_gain(v);
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
                    None => pad.local.as_ref().is_some_and(|l| l.pad.slot_active(i)),
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

// ---------- диалог обрезки ----------

use crate::mixer::SAMPLE_RATE;
use crate::ui::theme::pal;

enum Dlg {
    None,
    Cancel,
    Preview(usize, usize),
    Add,
}

/// Модальный диалог обрезки: волна, ручки начала/конца, прослушивание, «Добавить».
/// При `deaf` (в звонке с выключенным звуком) прослушивание заблокировано.
pub fn trim_dialog(ctx: &egui::Context, pad: &mut Soundpad, cfg: &mut Config, deaf: bool) {
    let Some(trim) = &mut pad.trim else { return };

    // Ответ фонового декода.
    if let Some(rx) = &trim.rx
        && let Ok(r) = rx.try_recv()
    {
        trim.busy = false;
        match r {
            Ok((source, peaks)) => {
                let dur = source.len() as f32 / SAMPLE_RATE as f32;
                if dur < 0.1 {
                    trim.error = Some("слишком короткий файл — нужно хотя бы 0,1 с".into());
                } else {
                    trim.source = source;
                    trim.peaks = peaks;
                    trim.start = 0.0;
                    trim.end = dur.min(import::MAX_CLIP_SECS);
                }
            }
            Err(e) => trim.error = Some(e),
        }
        trim.rx = None;
    }

    let title = trim.name.clone();
    let mut open = true;
    let mut action = Dlg::None;
    egui::Window::new(title)
        .id(egui::Id::unique("soundpad_trim"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .open(&mut open)
        .show(ctx, |ui| {
            let p = pal(ui);
            if trim.busy {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(RichText::new("Загружаю…").color(p.muted));
                });
                return;
            }
            if trim.source.is_empty() {
                if let Some(e) = &trim.error {
                    ui.label(RichText::new(e).color(p.err_text));
                }
                if ui.button("Закрыть").clicked() {
                    action = Dlg::Cancel;
                }
                return;
            }
            waveform(ui, trim, &p);
            ui.add_space(4.0);
            let len = trim.end - trim.start;
            ui.label(
                RichText::new(format!(
                    "{} → {} · длина {:.1} с (максимум {:.0} с)",
                    fmt_time(trim.start),
                    fmt_time(trim.end),
                    len,
                    import::MAX_CLIP_SECS
                ))
                .size(12.0)
                .color(p.dim),
            );
            if let Some(e) = &trim.error {
                ui.label(RichText::new(e).size(12.0).color(p.err_text));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!deaf, egui::Button::new("Прослушать"))
                    .on_hover_text(if deaf { "Включи звук, чтобы слушать" } else { "" })
                    .clicked()
                {
                    let s = (trim.start * SAMPLE_RATE as f32) as usize;
                    let e = (trim.end * SAMPLE_RATE as f32) as usize;
                    action = Dlg::Preview(s, e);
                }
                if ui.button("Отмена").clicked() {
                    action = Dlg::Cancel;
                }
                if ui.button(RichText::new("Добавить").color(p.accent)).clicked() {
                    action = Dlg::Add;
                }
            });
        });

    match action {
        Dlg::Preview(s, e) => {
            if let Some(trim) = &pad.trim {
                let clip = std::sync::Arc::new(trim.source[s.min(trim.source.len())..e.min(trim.source.len())].to_vec());
                if pad.local.is_none() {
                    match crate::soundpad::preview::LocalPad::start(cfg.output_device.as_deref()) {
                        Ok(l) => pad.local = Some(l),
                        Err(e) => {
                            if let Some(trim) = &mut pad.trim {
                                trim.error = Some(e);
                            }
                            return;
                        }
                    }
                }
                let local = pad.local.as_ref().unwrap();
                local.pad.set_gain(cfg.soundpad.volume);
                local.pad.play(usize::MAX, clip); // предпрослушивание — вне нумерации слотов
            }
        }
        Dlg::Add => {
            if let Some(dir) = import::sounds_dir() {
                let trim = pad.trim.as_ref().unwrap();
                let (slot, name) = (trim.target_slot, trim.name.clone());
                match pad.import_into(cfg, &dir, slot, name) {
                    Ok(()) => pad.trim = None,
                    Err(e) => {
                        if let Some(trim) = &mut pad.trim {
                            trim.error = Some(e);
                        }
                    }
                }
            }
        }
        Dlg::Cancel => pad.trim = None,
        Dlg::None => {}
    }
    if !open {
        pad.trim = None;
    }
}

/// Полоса волны с ручками начала и конца выбранного куска.
fn waveform(ui: &mut Ui, trim: &mut TrimState, p: &Palette) {
    let dur = trim.source.len() as f32 / SAMPLE_RATE as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(280.0), 80.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), p.surface);

    // Волна.
    let n = trim.peaks.len();
    if n > 0 {
        let yc = rect.center().y;
        for (i, &(lo, hi)) in trim.peaks.iter().enumerate() {
            let x = rect.left() + rect.width() * (i as f32 + 0.5) / n as f32;
            let h = ((hi - lo) / 2.0).max(0.003) * (rect.height() - 10.0) / 2.0;
            painter.line_segment([Pos2::new(x, yc - h), Pos2::new(x, yc + h)], Stroke::new(1.0, p.muted));
        }
    }

    // Выделение выбранного куска.
    let x0 = rect.left() + rect.width() * (trim.start / dur).clamp(0.0, 1.0);
    let x1 = rect.left() + rect.width() * (trim.end / dur).clamp(0.0, 1.0);
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom())),
        CornerRadius::same(4),
        p.accent.gamma_multiply(0.25),
    );

    // Ручки: тонкие вертикали, таскаются мышью.
    let clamp_range = |start: &mut f32, end: &mut f32| {
        *start = start.clamp(0.0, dur);
        *end = end.clamp(0.0, dur);
        if *end - *start > import::MAX_CLIP_SECS {
            *end = *start + import::MAX_CLIP_SECS;
        }
        if *end - *start < 0.1 {
            *end = (*start + 0.1).min(dur);
        }
    };
    let to_sec = |dx: f32| dx / rect.width() * dur;
    let h_start = ui.interact(Rect::from_min_max(Pos2::new(x0 - 5.0, rect.top()), Pos2::new(x0 + 5.0, rect.bottom())), ui.scope_id().with("trim_start"), Sense::drag());
    if h_start.dragged() {
        trim.start = (trim.start + to_sec(h_start.drag_delta().x)).min(trim.end - 0.1);
        clamp_range(&mut trim.start, &mut trim.end);
    }
    let h_end = ui.interact(Rect::from_min_max(Pos2::new(x1 - 5.0, rect.top()), Pos2::new(x1 + 5.0, rect.bottom())), ui.scope_id().with("trim_end"), Sense::drag());
    if h_end.dragged() {
        trim.end = (trim.end + to_sec(h_end.drag_delta().x)).max(trim.start + 0.1);
        clamp_range(&mut trim.start, &mut trim.end);
    }
    for (x, hovered) in [(x0, h_start.hovered() || h_start.dragged()), (x1, h_end.hovered() || h_end.dragged())] {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(if hovered { 3.0 } else { 2.0 }, p.text),
        );
    }
    if h_start.hovered() || h_end.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn fmt_time(sec: f32) -> String {
    format!("{}:{:04.1}", (sec as u32) / 60, sec % 60.0)
}

