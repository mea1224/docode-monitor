//! Native notification-area / menu-bar icon, owned by the UI thread.
#[derive(Debug, PartialEq)]
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
pub enum Action {
    Close,
    TogglePin,
}

#[cfg(any(windows, target_os = "macos"))]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
    pin: tray_icon::menu::MenuItem,
    events: std::sync::mpsc::Receiver<Action>,
}

#[cfg(any(windows, target_os = "macos"))]
impl Tray {
    pub fn new(ctx: &eframe::egui::Context) -> anyhow::Result<Self> {
        use tray_icon::{
            Icon, TrayIconBuilder,
            menu::{Menu, MenuEvent, MenuItem},
        };
        let menu = Menu::new();
        let close = MenuItem::new("关闭", true, None);
        let pin = MenuItem::new("固定", true, None);
        menu.append_items(&[&close, &pin])?;
        let image = crate::icon::tray();
        let icon = TrayIconBuilder::new()
            .with_tooltip("DoCode 余额与公告")
            .with_icon(Icon::from_rgba(image.rgba, image.width, image.height)?)
            .with_icon_as_template(false)
            .with_menu(Box::new(menu))
            .build()?;
        let (sender, events) = std::sync::mpsc::channel();
        let close_id = close.id().clone();
        let pin_id = pin.id().clone();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = if event.id == close_id {
                Action::Close
            } else if event.id == pin_id {
                Action::TogglePin
            } else {
                return;
            };
            let _ = sender.send(action);
            ctx.request_repaint();
        }));
        Ok(Self {
            _icon: icon,
            pin,
            events,
        })
    }

    pub fn next_action(&self) -> Option<Action> {
        self.events.try_recv().ok()
    }

    pub fn set_pinned(&self, pinned: bool) {
        self.pin.set_text(if pinned { "解除固定" } else { "固定" });
    }
}

#[cfg(any(windows, target_os = "macos"))]
impl Drop for Tray {
    fn drop(&mut self) {
        tray_icon::menu::MenuEvent::set_event_handler(None::<fn(tray_icon::menu::MenuEvent)>);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub struct Tray;

#[cfg(not(any(windows, target_os = "macos")))]
impl Tray {
    pub fn new(_: &eframe::egui::Context) -> anyhow::Result<Self> {
        Ok(Self)
    }
    pub fn next_action(&self) -> Option<Action> {
        None
    }
    pub fn set_pinned(&self, _: bool) {}
}
