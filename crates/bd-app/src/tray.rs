//! Tray icon with its quick menu.

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    AddFromClipboard,
    ToggleClipboard(bool),
    PauseAll,
    ResumeAll,
    /// Open the speed limit box of the window.
    SpeedLimit,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    add_clipboard: MenuItem,
    watch_clipboard: CheckMenuItem,
    pause_all: MenuItem,
    resume_all: MenuItem,
    speed: MenuItem,
    quit: MenuItem,
}

fn icon_image() -> Option<tray_icon::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../../../assets/tray-32.png")));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    tray_icon::Icon::from_rgba(buf, info.width, info.height).ok()
}

impl Tray {
    pub fn new(watch_clipboard: bool) -> Result<Self, String> {
        let open = MenuItem::new(t("Open BDownloader"), true, None);
        let add_clipboard = MenuItem::new(t("Add links from clipboard"), true, None);
        let watch = CheckMenuItem::new(t("Watch clipboard"), true, watch_clipboard, None);
        let pause_all = MenuItem::new(t("Pause all"), true, None);
        let resume_all = MenuItem::new(t("Resume all"), true, None);
        let speed = MenuItem::new(format!("{}…", t("Speed limit")), true, None);
        let quit = MenuItem::new(t("Quit"), true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &open,
            &PredefinedMenuItem::separator(),
            &add_clipboard,
            &watch,
            &PredefinedMenuItem::separator(),
            &pause_all,
            &resume_all,
            &speed,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|e| e.to_string())?;

        let mut builder =
            TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("BDownloader").with_menu_on_left_click(false);
        if let Some(image) = icon_image() {
            builder = builder.with_icon(image);
        }
        let icon = builder.build().map_err(|e| e.to_string())?;
        Ok(Tray { icon, open, add_clipboard, watch_clipboard: watch, pause_all, resume_all, speed, quit })
    }

    pub fn set_tooltip(&self, text: &str) {
        let _ = self.icon.set_tooltip(Some(text));
    }

    pub fn set_watch_clipboard(&self, on: bool) {
        self.watch_clipboard.set_checked(on);
    }

    /// Pending clicks on the icon and its menu.
    pub fn poll(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            match event {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => actions.push(TrayAction::Show),
                _ => {}
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id();
            if id == self.open.id() {
                actions.push(TrayAction::Show);
            } else if id == self.add_clipboard.id() {
                actions.push(TrayAction::AddFromClipboard);
            } else if id == self.watch_clipboard.id() {
                actions.push(TrayAction::ToggleClipboard(self.watch_clipboard.is_checked()));
            } else if id == self.pause_all.id() {
                actions.push(TrayAction::PauseAll);
            } else if id == self.resume_all.id() {
                actions.push(TrayAction::ResumeAll);
            } else if id == self.quit.id() {
                actions.push(TrayAction::Quit);
            } else if id == self.speed.id() {
                actions.push(TrayAction::SpeedLimit);
            }
        }
        actions
    }
}
