//! Палитры (тёмная и светлая), шрифты и настройка egui-стиля.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Theme,
};

const fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Цвета экрана. Берутся по `ui.visuals().dark_mode`.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub border: Color32,
    /// Рамка полей ввода.
    pub border_input: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Совсем тихий текст (подсказки, версия).
    pub dim: Color32,
    pub accent: Color32,
    /// Текст на кнопке акцентного цвета.
    pub on_accent: Color32,
    pub danger: Color32,
    pub warn: Color32,
    /// Дорожка полос и слайдеров.
    pub track: Color32,
    /// Фон круглых кнопок внизу.
    pub round_btn: Color32,
    pub round_btn_muted: Color32,
    pub err_bg: Color32,
    pub err_text: Color32,
    /// Заливка слайдера громкости.
    pub vol_fill: Color32,
    /// Затухающий «пик» на полосе VAD.
    pub vad_peak: Color32,
    pub avatars: [Color32; 7],
}

const DARK: Palette = Palette {
    bg: hex(0x15171B),
    surface: hex(0x1B1E23),
    border: hex(0x262A31),
    border_input: hex(0x2E333B),
    text: hex(0xE8EAED),
    muted: hex(0x9AA0A8),
    dim: hex(0x80868F),
    accent: hex(0x3DDC84),
    on_accent: hex(0x0D1F14),
    danger: hex(0xE5484D),
    warn: hex(0xF5A524),
    track: hex(0x2A2E35),
    round_btn: hex(0x262A31),
    round_btn_muted: hex(0x3A2224),
    err_bg: hex(0x2A1A1C),
    err_text: hex(0xFF9EA1),
    vol_fill: hex(0xC9CDD3),
    vad_peak: hex(0x1F3A2A),
    avatars: [
        hex(0x3A4A7A),
        hex(0x6B4A2E),
        hex(0x2E5B55),
        hex(0x6A3A5E),
        hex(0x55602F),
        hex(0x6B3A3A),
        hex(0x3A5F7A),
    ],
};

const LIGHT: Palette = Palette {
    bg: hex(0xF6F7F9),
    surface: hex(0xFFFFFF),
    border: hex(0xE1E4E8),
    border_input: hex(0xD3D7DD),
    text: hex(0x16181C),
    muted: hex(0x5F6670),
    dim: hex(0x7A828C),
    accent: hex(0x1F9D57),
    on_accent: hex(0xFFFFFF),
    danger: hex(0xE5484D),
    warn: hex(0xB7791F),
    track: hex(0xE1E4E8),
    round_btn: hex(0xE1E4E8),
    round_btn_muted: hex(0xFBDADB),
    err_bg: hex(0xFCE8E9),
    err_text: hex(0xB4232A),
    vol_fill: hex(0x5F6670),
    vad_peak: hex(0xB9E3CB),
    avatars: [
        hex(0xC9D4F5),
        hex(0xF2D9C4),
        hex(0xC5E6E1),
        hex(0xEBCDE0),
        hex(0xDCE5BE),
        hex(0xF0CCCC),
        hex(0xC4DDF0),
    ],
};

pub fn palette(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Семейство для полужирного начертания Inter.
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("InterSemiBold".into())
}

fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// Шрифт иконок Phosphor; строки-иконки (`egui_phosphor::regular::*`) рисуем только им.
pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icon_family())
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // egui-phosphor 0.14 собран под egui 0.36, поэтому берём только байты шрифта.
    fonts.font_data.insert("Inter".into(), FontData::from_static(include_bytes!("../../assets/Inter-Regular.ttf")).into());
    fonts.font_data.insert(
        "InterSemiBold".into(),
        FontData::from_static(include_bytes!("../../assets/Inter-SemiBold.ttf")).into(),
    );
    fonts.font_data.insert(
        "phosphor".into(),
        FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );

    // Inter первым, дальше встроенные шрифты egui как запас для редких символов. Иконки — в отдельном
    // семействе: в Inter те же кодовые точки приватной области заняты своими глифами (стрелки и т.п.),
    // а у Phosphor буквы a-z пустые, так что смешивать их в одном семействе нельзя.
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = vec!["Inter".to_string()];
    prop.extend(fallback.iter().cloned());
    let mut semi = vec!["InterSemiBold".to_string(), "Inter".to_string()];
    semi.extend(fallback);
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(semibold_family(), semi);
    fonts.families.insert(icon_family(), vec!["phosphor".to_string()]);
    ctx.set_fonts(fonts);
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border_input);
    v.window_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.surface;
    v.hyperlink_color = hex(0x8AB4FF);
    v.selection.bg_fill = p.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);

    let radius = CornerRadius::same(10);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for (s, fill) in [(&mut w.inactive, p.surface), (&mut w.hovered, p.surface), (&mut w.active, p.surface), (&mut w.open, p.surface)] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.corner_radius = radius;
        s.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border_input);
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.bg_stroke = Stroke::new(1.0, p.border_input);
    v
}

/// Один раз при старте: шрифты, размеры текста и обе палитры. Какую показывать — решает ОС
/// (`ThemePreference::System` по умолчанию).
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::ThemePreference::System);
    for (theme, dark) in [(Theme::Dark, true), (Theme::Light, false)] {
        ctx.set_visuals_of(theme, visuals(dark));
        ctx.style_mut_of(theme, |s| {
            s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
            s.text_styles.insert(TextStyle::Heading, semibold(20.0));
            s.spacing.item_spacing = egui::vec2(8.0, 8.0);
            s.spacing.button_padding = egui::vec2(12.0, 8.0);
            s.spacing.interact_size.y = 36.0;
            s.spacing.window_margin = Margin::same(10);
            s.spacing.menu_margin = Margin::same(6);
            s.spacing.scroll.bar_width = 6.0;
            s.spacing.scroll.floating = true;
        });
    }
}
