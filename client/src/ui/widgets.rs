//! Самодельные виджеты по макету: аватар, круглые кнопки, слайдер, переключатель, полосы.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2};

use super::theme::{Palette, semibold};

/// Детерминированный цвет аватара по хешу ника (FNV-1a).
pub fn avatar_color(nick: &str, pal: &Palette) -> Color32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in nick.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    pal.avatars[(h as usize) % pal.avatars.len()]
}

/// Круг 36 px с буквой ника; у говорящего — зелёное кольцо 2 px. Место под кольцо резервируется всегда.
pub fn avatar(ui: &mut Ui, nick: &str, speaking: bool, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    if speaking {
        p.circle_stroke(c, 21.0, Stroke::new(2.0, pal.accent));
    }
    p.circle_filled(c, 18.0, avatar_color(nick, pal));
    let initial = nick.chars().next().map(|ch| ch.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
    p.text(c, Align2::CENTER_CENTER, initial, semibold(15.0), pal.text);
}

/// Круглая кнопка с иконкой (52 px в нижней панели).
pub fn round_button(ui: &mut Ui, icon: &str, fill: Color32, fg: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        fill.gamma_multiply(0.8)
    } else if resp.hovered() {
        lighten(fill)
    } else {
        fill
    };
    ui.painter().circle_filled(rect.center(), 26.0, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(22.0), fg);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(tip)
}

fn lighten(c: Color32) -> Color32 {
    let m = |x: u8| x.saturating_add(14);
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// Иконка-кнопка без фона (шестерёнка, «назад»).
pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, color: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(10), ui.visuals().widgets.hovered.weak_bg_fill);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), color);
    resp.on_hover_text(tip)
}

/// Большая кнопка основного действия.
pub fn primary_button(ui: &mut Ui, icon: &str, text: &str, pal: &Palette) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        pal.accent.gamma_multiply(0.8)
    } else if resp.hovered() {
        lighten(pal.accent)
    } else {
        pal.accent
    };
    ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("{icon}  {text}"), semibold(15.0), pal.on_accent);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Логотип: пять столбиков волны (штрихи из макета, viewBox 24x24).
pub fn logo(ui: &mut Ui, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let k = size / 24.0;
    let stroke = Stroke::new(2.0 * k, color);
    for (x, y0, y1) in [(4.0, 10.0, 14.0), (8.0, 7.0, 17.0), (12.0, 4.0, 20.0), (16.0, 7.0, 17.0), (20.0, 10.0, 14.0)] {
        let a = rect.min + vec2(x * k, y0 * k);
        let b = rect.min + vec2(x * k, y1 * k);
        ui.painter().line_segment([a, b], stroke);
        ui.painter().circle_filled(a, k, color);
        ui.painter().circle_filled(b, k, color);
    }
}

/// Значок уровня связи: четыре столбика.
pub fn signal_icon(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let k = 14.0 / 24.0;
    for (x, top) in [(5.0, 15.0), (10.0, 12.0), (15.0, 9.0), (20.0, 6.0)] {
        let a = rect.min + vec2(x * k, top * k);
        let b = rect.min + vec2(x * k, 18.0 * k);
        ui.painter().line_segment([a, b], Stroke::new(2.4 * k, color));
    }
}

/// Тонкая полоса уровня (0..1).
pub fn level_bar(ui: &mut Ui, frac: f32, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), pal.track);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, CornerRadius::same(2), pal.accent);
}

/// Слайдер громкости 0..2 (0–200%) с шагом 5%; двойной клик — 100%. Возвращает true, если значение изменилось.
pub fn volume_slider(ui: &mut Ui, width: f32, value: &mut f32, pal: &Palette) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(30.0), 18.0), Sense::click_and_drag());
    let inset = 7.0;
    let (l, r) = (rect.left() + inset, rect.right() - inset);
    let mut changed = false;
    if resp.double_clicked() {
        *value = 1.0;
        changed = true;
    } else if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on() || resp.dragged()) {
        let v = (((pos.x - l) / (r - l)) * 2.0).clamp(0.0, 2.0);
        let v = (v * 20.0).round() / 20.0;
        if (v - *value).abs() > 1e-4 {
            *value = v;
            changed = true;
        }
    }
    let y = rect.center().y;
    let x = l + (r - l) * (*value / 2.0).clamp(0.0, 1.0);
    let track = Rect::from_min_max(Pos2::new(l, y - 2.0), Pos2::new(r, y + 2.0));
    ui.painter().rect_filled(track, CornerRadius::same(2), pal.track);
    ui.painter().rect_filled(Rect::from_min_max(track.min, Pos2::new(x, y + 2.0)), CornerRadius::same(2), pal.vol_fill);
    let knob = if resp.hovered() || resp.dragged() { 7.5 } else { 7.0 };
    ui.painter().circle_filled(Pos2::new(x, y), knob, pal.text);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    changed
}

/// Переключатель 40x24. Возвращает true при щелчке.
pub fn toggle(ui: &mut Ui, on: &mut bool, pal: &Palette) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 24.0), Sense::click());
    let clicked = resp.clicked();
    if clicked {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = pal.track.lerp_to_gamma(pal.accent, t);
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    let knob_col = if t > 0.5 { pal.on_accent } else { pal.muted };
    let x = rect.left() + 12.0 + 16.0 * t;
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 9.0, knob_col);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    clicked
}

/// Полоса голосовой активации: текущая вероятность речи, затухающий пик и перетаскиваемая
/// белая черта порога. Возвращает true, пока порог меняется; `threshold` ограничен `range`.
pub fn vad_bar(ui: &mut Ui, prob: f32, peak: f32, threshold: &mut f32, range: (f32, f32), pal: &Palette) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    let round = CornerRadius::same(6);
    ui.painter().rect_filled(rect, round, pal.surface);
    let inner = rect.shrink(1.0);
    let part = |f: f32| {
        let mut r = inner;
        r.set_width(inner.width() * f.clamp(0.0, 1.0));
        r
    };
    let inner_round = CornerRadius::same(5);
    ui.painter().rect_filled(part(peak), inner_round, pal.vad_peak);
    ui.painter().rect_filled(part(prob), inner_round, pal.accent);
    ui.painter().rect_stroke(rect, round, Stroke::new(1.0, pal.border_input), StrokeKind::Inside);

    if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on() || resp.dragged()) {
        let v = ((pos.x - rect.left()) / rect.width()).clamp(range.0, range.1);
        if (v - *threshold).abs() > 1e-4 {
            *threshold = v;
            resp.mark_changed();
        }
    }
    let x = rect.left() + rect.width() * threshold.clamp(0.0, 1.0);
    let line = Rect::from_min_max(Pos2::new(x - 1.5, rect.top() - 2.0), Pos2::new(x + 1.5, rect.bottom() + 2.0));
    ui.painter().rect_filled(line, CornerRadius::same(1), pal.text);
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    resp
}
