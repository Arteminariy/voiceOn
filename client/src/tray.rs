//! Значок в трее: на Linux — StatusNotifierItem по D-Bus (ksni, без GTK),
//! на Windows — `tray-icon` в цикле сообщений winit главного потока.
//!
//! Действия из трея приходят в канал, а контекст egui будится `request_repaint`,
//! чтобы `App::logic` обработал их, даже когда окно спрятано.

use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Show,
    ToggleMic,
    ToggleSound,
    Quit,
    /// Хост трея (панель) пропал или появился снова — только Linux.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    HostOffline,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    HostOnline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    pub in_call: bool,
    pub mic_muted: bool,
    pub deafened: bool,
}

impl State {
    /// 0 — серый (не в звонке), 1 — зелёный (в звонке), 2 — красный (микрофон молчит).
    fn color_index(self) -> usize {
        match self {
            State { in_call: false, .. } => 0,
            State { mic_muted: true, .. } | State { deafened: true, .. } => 2,
            _ => 1,
        }
    }

    fn tooltip(self) -> &'static str {
        match self.color_index() {
            0 => "не в звонке",
            1 => "в звонке",
            _ => "в звонке, микрофон выключен",
        }
    }

    fn mic_label(self) -> &'static str {
        if self.mic_muted { "Микрофон: выкл" } else { "Микрофон: вкл" }
    }

    fn sound_label(self) -> &'static str {
        if self.deafened { "Звук: выкл" } else { "Звук: вкл" }
    }
}

const COLORS: [[u8; 3]; 3] = [[0x9A, 0xA0, 0xA8], [0x3D, 0xDC, 0x84], [0xE5, 0x48, 0x4D]];
/// Цвета исходной иконки: фон плитки и зелёные штрихи (см. assets/gen_icon.py).
const ICON_BG: [u8; 3] = [21, 23, 27];
const ICON_FG_G: u8 = 220;

fn source_icon() -> &'static (Vec<u8>, u32, u32) {
    static ICON: OnceLock<(Vec<u8>, u32, u32)> = OnceLock::new();
    ICON.get_or_init(|| {
        let d = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("встроенная иконка");
        (d.rgba, d.width, d.height)
    })
}

/// Иконка `size`×`size` в RGBA: штрихи перекрашены в цвет `color`, фон как был.
fn icon_rgba(color: [u8; 3], size: u32) -> Vec<u8> {
    let (src, w, h) = source_icon();
    let (w, h) = (*w as usize, *h as usize);
    // Перекраска: доля «зелени» пикселя по каналу G между фоном и штрихом.
    let span = (ICON_FG_G - ICON_BG[1]) as f32;
    let tinted: Vec<[f32; 4]> = src
        .chunks_exact(4)
        .map(|p| {
            let t = ((p[1] as f32 - ICON_BG[1] as f32) / span).clamp(0.0, 1.0);
            let a = p[3] as f32 / 255.0;
            let c = |i: usize| (ICON_BG[i] as f32 + (color[i] as f32 - ICON_BG[i] as f32) * t) * a;
            [c(0), c(1), c(2), a] // предумноженная альфа — без тёмной каймы при уменьшении
        })
        .collect();
    // Уменьшение усреднением блоков.
    let size = size as usize;
    let mut out = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        let (y0, y1) = (y * h / size, ((y + 1) * h / size).max(y * h / size + 1));
        for x in 0..size {
            let (x0, x1) = (x * w / size, ((x + 1) * w / size).max(x * w / size + 1));
            let mut acc = [0f32; 4];
            for yy in y0..y1 {
                for p in &tinted[yy * w + x0..yy * w + x1] {
                    for i in 0..4 {
                        acc[i] += p[i];
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f32;
            let a = acc[3] / n;
            let un = |v: f32| if a > 0.0 { (v / n / a).round().clamp(0.0, 255.0) as u8 } else { 0 };
            out.extend_from_slice(&[un(acc[0]), un(acc[1]), un(acc[2]), (a * 255.0).round() as u8]);
        }
    }
    out
}

pub struct Tray {
    rx: Receiver<Event>,
    state: State,
    imp: imp::Imp,
}

impl Tray {
    pub fn spawn(ctx: &egui::Context) -> Result<Tray, String> {
        let (tx, rx) = mpsc::channel();
        let state = State::default();
        let imp = imp::Imp::spawn(Notify { tx, ctx: ctx.clone() }, state)?;
        Ok(Tray { rx, state, imp })
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Обновляет значок и меню, только если состояние поменялось.
    pub fn set_state(&mut self, state: State) {
        if state != self.state {
            self.state = state;
            self.imp.set_state(state);
        }
    }
}

/// Отправка события в приложение с пробуждением egui.
#[derive(Clone)]
struct Notify {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Notify {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{MenuItem, StandardItem};

    use super::{COLORS, Event, Notify, State, icon_rgba};

    struct KTray {
        notify: Notify,
        state: State,
        icons: [Vec<ksni::Icon>; 3],
    }

    fn argb(color: [u8; 3], size: u32) -> ksni::Icon {
        let mut data = icon_rgba(color, size);
        for p in data.chunks_exact_mut(4) {
            p.rotate_right(1); // RGBA -> ARGB (сетевой порядок байт)
        }
        ksni::Icon { width: size as i32, height: size as i32, data }
    }

    impl ksni::Tray for KTray {
        fn id(&self) -> String {
            "voiceon".into()
        }
        fn title(&self) -> String {
            "voiceOn".into()
        }
        fn category(&self) -> ksni::Category {
            ksni::Category::Communications
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons[self.state.color_index()].clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "voiceOn".into(),
                description: self.state.tooltip().into(),
                ..Default::default()
            }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            self.notify.send(Event::Show);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, enabled: bool, e: Event| -> MenuItem<Self> {
                StandardItem {
                    label: label.into(),
                    enabled,
                    activate: Box::new(move |t: &mut Self| t.notify.send(e)),
                    ..Default::default()
                }
                .into()
            };
            let call = self.state.in_call;
            vec![
                item("Показать окно", true, Event::Show),
                MenuItem::Separator,
                item(self.state.mic_label(), call, Event::ToggleMic),
                item(self.state.sound_label(), call, Event::ToggleSound),
                MenuItem::Separator,
                item("Выход", true, Event::Quit),
            ]
        }
        fn watcher_online(&self) {
            self.notify.send(Event::HostOnline);
        }
        fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
            crate::log::info(&format!("трей: панель недоступна ({reason:?})"));
            self.notify.send(Event::HostOffline);
            true // ждём, пока панель появится снова
        }
    }

    pub struct Imp(Handle<KTray>);

    impl Imp {
        pub fn spawn(notify: Notify, state: State) -> Result<Imp, String> {
            let icons = COLORS.map(|c| vec![argb(c, 32), argb(c, 64)]);
            let tray = KTray { notify, state, icons };
            // Панель может ещё не подняться (автозапуск при входе) — не считаем это ошибкой.
            let handle = tray.assume_sni_available(true).spawn().map_err(|e| e.to_string())?;
            Ok(Imp(handle))
        }

        pub fn set_state(&mut self, state: State) {
            self.0.update(|t| t.state = state);
        }
    }
}

#[cfg(windows)]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{COLORS, Event, Notify, State, icon_rgba};

    pub struct Imp {
        tray: TrayIcon,
        mic: MenuItem,
        sound: MenuItem,
        icons: [Icon; 3],
    }

    fn icon(color: [u8; 3]) -> Result<Icon, String> {
        Icon::from_rgba(icon_rgba(color, 32), 32, 32).map_err(|e| e.to_string())
    }

    impl Imp {
        pub fn spawn(notify: Notify, state: State) -> Result<Imp, String> {
            let icons = [icon(COLORS[0])?, icon(COLORS[1])?, icon(COLORS[2])?];
            let show = MenuItem::with_id("show", "Показать окно", true, None);
            let mic = MenuItem::with_id("mic", state.mic_label(), state.in_call, None);
            let sound = MenuItem::with_id("sound", state.sound_label(), state.in_call, None);
            let quit = MenuItem::with_id("quit", "Выход", true, None);
            let menu = Menu::new();
            menu.append_items(&[&show, &PredefinedMenuItem::separator(), &mic, &sound, &PredefinedMenuItem::separator(), &quit])
                .map_err(|e| e.to_string())?;

            // Обработчики зовутся из цикла сообщений главного потока; будим egui сами,
            // иначе при спрятанном окне событие ждало бы следующей перерисовки.
            let n = notify.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let ev = match e.id() {
                    id if *id == MenuId::new("show") => Event::Show,
                    id if *id == MenuId::new("mic") => Event::ToggleMic,
                    id if *id == MenuId::new("sound") => Event::ToggleSound,
                    id if *id == MenuId::new("quit") => Event::Quit,
                    _ => return,
                };
                n.send(ev);
            }));
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                let show = matches!(
                    e,
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                        | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
                );
                if show {
                    notify.send(Event::Show);
                }
            }));

            let tray = TrayIconBuilder::new()
                .with_id("voiceon")
                .with_tooltip(format!("voiceOn — {}", state.tooltip()))
                .with_icon(icons[state.color_index()].clone())
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Imp { tray, mic, sound, icons })
        }

        pub fn set_state(&mut self, state: State) {
            let _ = self.tray.set_icon(Some(self.icons[state.color_index()].clone()));
            let _ = self.tray.set_tooltip(Some(format!("voiceOn — {}", state.tooltip())));
            self.mic.set_text(state.mic_label());
            self.mic.set_enabled(state.in_call);
            self.sound.set_text(state.sound_label());
            self.sound.set_enabled(state.in_call);
        }
    }

    impl Drop for Imp {
        fn drop(&mut self) {
            // Убираем значок сразу, иначе в трее остаётся «призрак» до наведения мыши.
            let _ = self.tray.set_visible(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tinted_icon_sizes_and_colors() {
        for size in [32u32, 64] {
            let px = icon_rgba(COLORS[2], size);
            assert_eq!(px.len(), (size * size * 4) as usize);
            // Центр иконки — средний штрих: должен стать красным.
            let c = ((size / 2 * size + size / 2) * 4) as usize;
            assert!(px[c] > 180 && px[c + 1] < 120, "{:?}", &px[c..c + 4]);
            // Угол — прозрачный.
            assert_eq!(px[3], 0);
        }
    }

    #[test]
    fn color_index() {
        let s = |in_call, mic_muted, deafened| State { in_call, mic_muted, deafened }.color_index();
        assert_eq!(s(false, true, true), 0);
        assert_eq!(s(true, false, false), 1);
        assert_eq!(s(true, true, false), 2);
        assert_eq!(s(true, false, true), 2);
    }
}
