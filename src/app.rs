use crate::{
    desktop::Desktop,
    fonts,
    model::{Announcement, Balance, Credentials, User, quota_units},
    worker::{self, Command, Event},
};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use raw_window_handle::HasWindowHandle;
use std::{
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};

const WIDTH: f32 = 380.0;
const WHITE: Color32 = Color32::from_rgb(240, 242, 246);
const MUTED: Color32 = Color32::from_rgb(144, 150, 162);
const ACCENT: Color32 = Color32::from_rgb(142, 191, 181);

pub struct DocodeApp {
    credentials: Credentials,
    user: Option<User>,
    balance: Option<Balance>,
    announcements: Vec<Announcement>,
    commands: Option<Sender<Command>>,
    events: Option<Receiver<Event>>,
    booting: bool,
    logging_in: bool,
    balance_loading: bool,
    announcements_loading: bool,
    login_error: Option<String>,
    balance_error: Option<String>,
    announcement_error: Option<String>,
    notice: Option<String>,
    storage_path: String,
    expanded: bool,
    desktop: Desktop,
    tray: Option<crate::tray::Tray>,
    demo: bool,
    demo_refresh: Option<Instant>,
    last_height: f32,
    capture: Option<crate::capture::Capture>,
    capture_scroll: Option<f32>,
    capture_right_click: Option<Pos2>,
}

impl DocodeApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        demo: bool,
        capture: Option<std::path::PathBuf>,
    ) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let has_font = fonts::install(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::TRANSPARENT;
        style.visuals.override_text_color = Some(WHITE);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(33, 35, 41);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(49, 53, 62);
        style.visuals.selection.bg_fill = Color32::from_rgb(57, 95, 88);
        style.visuals.widgets.inactive.corner_radius = 7.into();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 7.0);
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        cc.egui_ctx.set_style(style);
        let (commands, events) = if demo {
            (None, None)
        } else {
            let (commands, events) = worker::start(cc.egui_ctx.clone());
            (Some(commands), Some(events))
        };
        let (tray, tray_error) = match crate::tray::Tray::new(&cc.egui_ctx) {
            Ok(tray) => (Some(tray), None),
            Err(error) => (None, Some(format!("创建托盘图标失败：{error:#}"))),
        };
        let (desktop, desktop_error) =
            match Desktop::new(cc.window_handle().ok().map(|handle| handle.as_raw())) {
                Ok(desktop) => (desktop, None),
                Err(error) => (
                    Desktop::new(None).expect("headless desktop"),
                    Some(format!("初始化窗口失败：{error:#}")),
                ),
            };
        Self {
            credentials: Credentials { username: String::new(), password: String::new() },
            user: demo.then(|| User { id: 1, username: "demo".into(), display_name: "演示用户".into(), group: "vip1000".into() }),
            balance: demo.then(demo_balance),
            announcements: if demo { demo_announcements() } else { Vec::new() },
            commands, events, booting: !demo, logging_in: false, balance_loading: false,
            announcements_loading: false, login_error: None, balance_error: None,
            announcement_error: None,
            notice: desktop_error.or(tray_error).or_else(|| (!has_font).then(|| "Chinese font missing. Install Noto Sans CJK or set DOCODE_FONT to a CJK font file.".into())),
            storage_path: String::new(), expanded: false,
            desktop,
            tray,
            demo, demo_refresh: None, last_height: 0.0,
            capture: capture.map(crate::capture::Capture::new), capture_scroll: None,
            capture_right_click: None,
        }
    }

    fn send(&mut self, command: Command) -> bool {
        if let Some(sender) = &self.commands
            && sender.send(command).is_ok()
        {
            return true;
        }
        self.notice = Some("后台服务已停止，请重新启动程序".into());
        false
    }

    fn login(&mut self) {
        if self.logging_in || self.booting {
            return;
        }
        self.credentials.username = self.credentials.username.trim().to_owned();
        if self.credentials.username.is_empty() || self.credentials.password.is_empty() {
            self.login_error = Some("请输入用户名和密码".into());
            return;
        }
        self.login_error = None;
        self.logging_in = self.send(Command::Login(self.credentials.clone()));
    }

    fn refresh(&mut self) {
        if self.balance_loading || self.announcements_loading {
            return;
        }
        self.balance = None;
        self.balance_error = None;
        self.announcements.clear();
        self.announcement_error = None;
        if self.demo {
            self.balance_loading = true;
            self.announcements_loading = true;
            self.demo_refresh = Some(Instant::now() + Duration::from_millis(800));
        } else {
            self.balance_loading = self.send(Command::Balance);
            self.announcements_loading = self.send(Command::Announcements);
        }
    }

    fn receive(&mut self) {
        while let Some(event) = self
            .events
            .as_ref()
            .and_then(|events| events.try_recv().ok())
        {
            match event {
                Event::Ready {
                    credentials,
                    notice,
                    path,
                } => {
                    self.booting = false;
                    self.storage_path = path;
                    if notice.is_some() {
                        self.notice = notice;
                    }
                    if let Some(credentials) = credentials {
                        self.credentials = credentials;
                        self.login();
                    }
                }
                Event::StorageError(error) => {
                    self.booting = false;
                    self.notice = Some(error);
                }
                Event::Login(result) => {
                    self.logging_in = false;
                    match result {
                        Ok(user) => {
                            self.user = Some(user);
                            self.login_error = None;
                            self.refresh();
                        }
                        Err(error) => self.login_error = Some(error),
                    }
                }
                Event::Balance(result) => {
                    self.balance_loading = false;
                    match result {
                        Ok(balance) => {
                            if let Some(user) = &mut self.user {
                                user.group = balance.group.clone();
                            }
                            self.balance = Some(balance);
                        }
                        Err(error) => self.balance_error = Some(error),
                    }
                }
                Event::Announcements(result) => {
                    self.announcements_loading = false;
                    match result {
                        Ok(announcements) => {
                            self.announcements = announcements;
                            self.announcement_error = None;
                        }
                        Err(error) => self.announcement_error = Some(error),
                    }
                }
            }
        }
        if self.demo_refresh.is_some_and(|when| Instant::now() >= when) {
            self.demo_refresh = None;
            self.balance_loading = false;
            self.balance = Some(demo_balance());
            self.announcements_loading = false;
            self.announcements = demo_announcements();
        }
    }

    fn drag_area(&self, ui: &mut egui::Ui, rect: Rect, id: &'static str) {
        self.desktop.drag_area(ui, rect, id);
    }

    fn login_ui(&mut self, ui: &mut egui::Ui) {
        let top = ui.cursor().min;
        self.drag_area(
            ui,
            Rect::from_min_size(top, Vec2::new(ui.available_width(), 54.0)),
            "login-drag",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("docode").size(23.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Close, true)
                    .on_hover_text("关闭")
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
        ui.label(
            RichText::new("登录后，余额与动态就在桌面")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(18.0);
        ui.add_enabled_ui(!self.logging_in && !self.booting, |ui| {
            ui.label(RichText::new("用户名").size(12.0).color(MUTED));
            ui.add_sized(
                [ui.available_width(), 32.0],
                egui::TextEdit::singleline(&mut self.credentials.username)
                    .hint_text("输入用户名")
                    .margin(Vec2::new(9.0, 7.0)),
            );
            ui.add_space(3.0);
            ui.label(RichText::new("密码").size(12.0).color(MUTED));
            let password = ui.add_sized(
                [ui.available_width(), 32.0],
                egui::TextEdit::singleline(&mut self.credentials.password)
                    .password(true)
                    .hint_text("输入密码")
                    .margin(Vec2::new(9.0, 7.0)),
            );
            ui.add_space(10.0);
            let submit = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::Button::new(RichText::new("登  录").color(Color32::from_rgb(15, 29, 27)))
                    .fill(ACCENT),
            );
            if submit.clicked()
                || (password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                self.login();
            }
        });
        if self.logging_in || self.booting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(if self.logging_in {
                        "正在登录…"
                    } else {
                        "正在读取本地凭据…"
                    })
                    .size(12.0)
                    .color(MUTED),
                );
            });
        } else {
            ui.label(
                RichText::new("凭据保存在本机，下次启动自动登录")
                    .size(11.0)
                    .color(MUTED),
            )
            .on_hover_text(&self.storage_path);
        }
        if let Some(error) = &self.login_error {
            error_box(ui, "login-error", error);
        }
    }

    fn dashboard_ui(&mut self, ui: &mut egui::Ui) {
        let top = ui.cursor().min;
        self.drag_area(
            ui,
            Rect::from_min_size(top, Vec2::new(ui.available_width(), 176.0)),
            "dashboard-drag",
        );
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 176.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(176.0);
                ui.horizontal(|ui| {
                    let user = self.user.as_ref().unwrap();
                    let refreshing = self.balance_loading || self.announcements_loading;
                    let display_name = if refreshing {
                        "—"
                    } else {
                        &user.display_name
                    };
                    let group = if refreshing { "—" } else { &user.group };
                    let name_width = ui
                        .fonts_mut(|fonts| {
                            fonts
                                .layout_no_wrap(
                                    display_name.to_owned(),
                                    FontId::proportional(15.0),
                                    WHITE,
                                )
                                .size()
                                .x
                        })
                        .clamp(20.0, 172.0);
                    ui.add_sized(
                        [name_width, 26.0],
                        egui::Label::new(RichText::new(display_name).size(15.0).strong())
                            .truncate(),
                    )
                    .on_hover_text(display_name);
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(37, 44, 44))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(7, 3))
                        .show(ui, |ui| {
                            let group_width = ui
                                .fonts_mut(|fonts| {
                                    fonts
                                        .layout_no_wrap(
                                            group.to_owned(),
                                            FontId::proportional(10.0),
                                            ACCENT,
                                        )
                                        .size()
                                        .x
                                })
                                .clamp(16.0, 76.0);
                            ui.add_sized(
                                [group_width, 14.0],
                                egui::Label::new(RichText::new(group).color(ACCENT).size(10.0))
                                    .truncate(),
                            );
                        });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(
                            ui,
                            Icon::Refresh,
                            !self.balance_loading && !self.announcements_loading,
                        )
                        .on_hover_text("刷新余额和公告")
                        .clicked()
                        {
                            self.refresh();
                        }
                    });
                });
                ui.add_space(19.0);
                let (quota, used, requests) = self
                    .balance
                    .as_ref()
                    .map(|balance| {
                        (
                            quota_units(balance.quota),
                            quota_units(balance.used_quota),
                            balance.request_count.to_string(),
                        )
                    })
                    .unwrap_or_else(|| ("—".into(), "—".into(), "—".into()));
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    for (amount, tooltip, color, base_size) in [
                        (quota, "剩余余额", WHITE, 36.0),
                        (used, "已使用的余额", MUTED, 24.0),
                    ] {
                        let width = ui.fonts_mut(|fonts| {
                            fonts
                                .layout_no_wrap(
                                    amount.clone(),
                                    FontId::proportional(base_size),
                                    color,
                                )
                                .size()
                                .x
                        });
                        let size = base_size * (ui.available_width() / width).min(1.0);
                        ui.label(RichText::new(amount).size(size).color(color))
                            .on_hover_text(tooltip);
                    }
                    ui.label(
                        RichText::new(if self.balance.is_some() {
                            format!("已请求：{requests}次")
                        } else {
                            "—".into()
                        })
                        .size(12.0)
                        .color(MUTED),
                    );
                });
            },
        );
        let (line, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment(
            [line.left_center(), line.right_center()],
            Stroke::new(0.7, Color32::from_white_alpha(90)),
        );
        ui.add_space(5.0);
        if let Some(error) = &self.balance_error {
            error_box(ui, "balance-error", error);
        }
        let announcement_height = if self.expanded {
            264.0
        } else if self.announcement_error.is_some() {
            120.0
        } else {
            60.0
        };
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), announcement_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_height(announcement_height);
                ui.set_max_height(announcement_height);
                if self.announcements_loading {
                    ui.label(RichText::new("—").color(MUTED).size(12.0));
                } else if let Some(error) = &self.announcement_error {
                    ui.label(RichText::new("—").color(MUTED).size(12.0));
                    error_box(ui, "announcement-error", error);
                    if ui.small_button("重试公告").clicked() {
                        self.announcement_error = None;
                        self.announcements_loading = self.send(Command::Announcements);
                    }
                } else if self.announcements.is_empty() {
                    ui.label(RichText::new("暂无公告").size(12.0).color(MUTED));
                } else if self.expanded {
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_salt("announcements")
                        .max_height(announcement_height)
                        .auto_shrink([false, false]);
                    if let Some(offset) = self.capture_scroll.take() {
                        scroll = scroll.vertical_scroll_offset(offset);
                    }
                    scroll.show(ui, |ui| {
                        for (index, announcement) in self.announcements.iter().enumerate() {
                            timeline_item(
                                ui,
                                announcement,
                                index + 1 != self.announcements.len(),
                                false,
                            );
                        }
                    });
                } else {
                    let rect = Rect::from_min_size(
                        ui.cursor().min,
                        Vec2::new(ui.available_width(), announcement_height),
                    );
                    let mut preview = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    preview.set_clip_rect(rect);
                    timeline_item(&mut preview, &self.announcements[0], false, true);
                    // Fade the bottom of the fixed-height preview into the card.
                    for offset in 0..18 {
                        let y = rect.bottom() - 18.0 + offset as f32;
                        preview.painter().rect_filled(
                            Rect::from_min_size(
                                Pos2::new(rect.left(), y),
                                Vec2::new(rect.width(), 1.0),
                            ),
                            0,
                            Color32::from_rgba_unmultiplied(15, 17, 22, (offset * 12) as u8),
                        );
                    }
                }
            },
        );
        ui.vertical_centered(|ui| {
            if icon_button(ui, if self.expanded { Icon::Up } else { Icon::Down }, true)
                .on_hover_text(if self.expanded {
                    "收起公告"
                } else {
                    "展开历史公告"
                })
                .clicked()
            {
                self.expanded = !self.expanded;
            }
        });
        if self.demo {
            ui.label(
                RichText::new("演示模式 · 不连接服务器")
                    .size(10.0)
                    .color(MUTED),
            );
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("关闭").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            ui.close();
        }
        let pin_label = if self.desktop.pinned {
            "解除固定"
        } else {
            "固定"
        };
        if ui.button(pin_label).clicked() {
            self.toggle_pin(ui.ctx());
            ui.close();
        }
    }

    fn toggle_pin(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.desktop.set_pinned(!self.desktop.pinned, ctx) {
            self.notice = Some(format!("设置窗口固定状态失败：{error:#}"));
        }
        if let Some(tray) = &self.tray {
            tray.set_pinned(self.desktop.pinned);
        }
    }
}

impl eframe::App for DocodeApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(pos) = self.capture_right_click.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            for pressed in [true, false] {
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive();
        while let Some(action) = self.tray.as_ref().and_then(|tray| tray.next_action()) {
            match action {
                crate::tray::Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                crate::tray::Action::TogglePin => self.toggle_pin(ctx),
            }
        }
        if let Some(mut capture) = self.capture.take() {
            match capture.tick(ctx) {
                Ok(true) => match capture.step {
                    1 => self.expanded = true,
                    2 => self.capture_scroll = Some(140.0),
                    3 => self.refresh(),
                    4 => {
                        self.balance = Some(demo_balance());
                        self.balance_loading = false;
                        self.announcements_loading = false;
                        self.announcements = demo_announcements();
                        self.demo_refresh = None;
                        let result = self.desktop.set_pinned(true, ctx);
                        capture
                            .report
                            .push(format!("pin below application windows: {result:?}"));
                        if let Err(error) = result {
                            self.notice = Some(error.to_string());
                        }
                    }
                    5 => {
                        let result = self.desktop.set_pinned(false, ctx);
                        capture
                            .report
                            .push(format!("unpin to topmost window level: {result:?}"));
                        self.expanded = false;
                        self.capture_scroll = None;
                    }
                    6 => {
                        self.user = None;
                        self.notice = None;
                    }
                    7 => self.capture_right_click = Some(Pos2::new(100.0, 130.0)),
                    _ => {}
                },
                Ok(false) => {}
                Err(error) => {
                    self.notice = Some(format!("截图失败：{error:#}"));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            self.capture = Some(capture);
        }
        let height = if self.user.is_some() {
            344.0
                + if self.expanded { 204.0 } else { 0.0 }
                + if self.balance_error.is_some() {
                    60.0
                } else {
                    0.0
                }
                + if self.demo { 20.0 } else { 0.0 }
                + if !self.expanded && self.announcement_error.is_some() {
                    60.0
                } else {
                    0.0
                }
        } else {
            354.0
                + if self.login_error.is_some() {
                    62.0
                } else {
                    0.0
                }
        } + if self.notice.is_some() { 66.0 } else { 0.0 };
        if self.last_height != height {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(WIDTH, height)));
            self.last_height = height;
        }
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(15, 17, 22, 244))
            .stroke(Stroke::new(1.0, Color32::from_white_alpha(32)))
            .corner_radius(18)
            .inner_margin(20);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            if self.user.is_some() {
                self.dashboard_ui(ui);
            } else {
                self.login_ui(ui);
            }
            if let Some(notice) = self.notice.clone() {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 30.0);
                        error_box(ui, "notice", &notice);
                    });
                    if icon_button(ui, Icon::Close, true)
                        .on_hover_text("关闭提示")
                        .clicked()
                    {
                        self.notice = None;
                    }
                });
            }
        });
        window_context_menu(ctx, |ui| self.menu(ui));
        // Use the state after the menu action, so pinning cannot queue a focus
        // request that would raise the window again in the same frame.
        self.desktop.focus_on_pointer_press(ctx);
        if self.balance_loading || self.logging_in || self.announcements_loading {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }
}

/// Observe the viewport input instead of a background widget response: child
/// controls (including the scroll area and drag region) must not steal the menu.
fn window_context_menu<R>(
    ctx: &egui::Context,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let viewport = ctx.viewport_rect();
    let right_clicked = ctx.input(|input| {
        input.pointer.button_clicked(egui::PointerButton::Secondary)
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| viewport.contains(pos))
    });
    egui::Popup::new(
        egui::Id::new("window-context-menu"),
        ctx.clone(),
        egui::PopupAnchor::PointerFixed,
        egui::LayerId::background(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .open_memory(right_clicked.then_some(egui::SetOpenCommand::Bool(true)))
    .close_behavior(if right_clicked {
        egui::PopupCloseBehavior::IgnoreClicks
    } else {
        egui::PopupCloseBehavior::CloseOnClick
    })
    .show(content)
}

fn error_box(ui: &mut egui::Ui, id: &'static str, message: &str) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(48.0)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(message)
                        .size(11.0)
                        .color(Color32::from_rgb(229, 175, 151)),
                )
                .wrap(),
            );
        });
}

fn timeline_item(ui: &mut egui::Ui, announcement: &Announcement, continues: bool, muted: bool) {
    let start = ui.cursor().min;
    let width = ui.available_width();
    let color = if muted {
        Color32::from_rgb(134, 140, 152)
    } else {
        Color32::from_rgb(205, 209, 217)
    };
    let response = ui.horizontal_top(|ui| {
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width((width - 24.0).max(10.0));
            ui.add(
                egui::Label::new(RichText::new(&announcement.content).size(12.0).color(color))
                    .wrap(),
            );
            ui.label(
                RichText::new(announcement.timestamp())
                    .size(10.0)
                    .color(MUTED),
            );
            ui.add_space(12.0);
        });
    });
    let point = Pos2::new(start.x + 4.0, start.y + 7.0);
    ui.painter()
        .circle_filled(point, 3.1, Color32::from_rgb(133, 143, 153));
    if continues {
        ui.painter().line_segment(
            [
                point + Vec2::new(0.0, 8.0),
                Pos2::new(point.x, response.response.rect.bottom() - 1.0),
            ],
            Stroke::new(0.7, Color32::from_gray(70)),
        );
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Refresh,
    Up,
    Down,
    Close,
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(26.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let center = rect.center();
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, 7, Color32::from_white_alpha(15));
    }
    let stroke = Stroke::new(1.5, if enabled { WHITE } else { MUTED });
    match icon {
        Icon::Close => {
            ui.painter().line_segment(
                [center + Vec2::new(-4.0, -4.0), center + Vec2::new(4.0, 4.0)],
                stroke,
            );
            ui.painter().line_segment(
                [center + Vec2::new(-4.0, 4.0), center + Vec2::new(4.0, -4.0)],
                stroke,
            );
        }
        Icon::Up | Icon::Down => {
            let direction = if matches!(icon, Icon::Down) {
                1.0
            } else {
                -1.0
            };
            ui.painter().add(egui::Shape::line(
                vec![
                    center + Vec2::new(-5.0, -2.0 * direction),
                    center + Vec2::new(0.0, 3.0 * direction),
                    center + Vec2::new(5.0, -2.0 * direction),
                ],
                stroke,
            ));
        }
        Icon::Refresh => {
            let angle = if enabled {
                0.0
            } else {
                ui.input(|i| i.time) as f32 * 4.0
            };
            let points: Vec<_> = (0..=24)
                .map(|step| {
                    let a = angle - 0.8 + step as f32 / 24.0 * 5.1;
                    center + Vec2::angled(a) * 6.0
                })
                .collect();
            let end = *points.last().unwrap();
            ui.painter().add(egui::Shape::line(points, stroke));
            ui.painter().add(egui::Shape::line(
                vec![
                    end + Vec2::angled(angle + 0.2) * 3.5,
                    end,
                    end + Vec2::angled(angle - 1.2) * 3.5,
                ],
                stroke,
            ));
        }
    }
    response
}

fn demo_balance() -> Balance {
    Balance {
        quota: 6_532_248_664,
        used_quota: 45_117_751_336,
        request_count: 21219,
        group: "vip1000".into(),
    }
}
fn demo_announcements() -> Vec<Announcement> {
    [
        (
            "gemini 3 系、4 系服务暂时不可用，恢复后会第一时间通知。",
            "2026-11-14T18:54:11.543Z",
        ),
        (
            "Gemini 按次额度调整，请留意最新分组价格。",
            "2026-11-12T14:41:26.632Z",
        ),
        (
            "Claude 特殊渠道倍率上调，详情请查看站点公告。",
            "2026-11-08T12:56:07.877Z",
        ),
        (
            "Gemini Ultra 分组上架，提供高中低三档模型。",
            "2026-10-22T08:08:36.769Z",
        ),
        (
            "生图功能已恢复。\n感谢大家的耐心等待。",
            "2026-10-15T02:56:28.283Z",
        ),
        (
            "更多历史公告会在此处按接口原始顺序展示，滚动时余额区域保持固定。",
            "2026-10-09T01:45:38.025Z",
        ),
    ]
    .into_iter()
    .map(|(content, publish_date)| Announcement {
        content: content.into(),
        publish_date: publish_date.into(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_channels() -> (DocodeApp, Receiver<Command>, Sender<Event>) {
        let (commands, received_commands) = std::sync::mpsc::channel();
        let (sent_events, events) = std::sync::mpsc::channel();
        (
            DocodeApp {
                credentials: Credentials {
                    username: String::new(),
                    password: String::new(),
                },
                user: Some(User {
                    id: 1,
                    username: "demo".into(),
                    display_name: "Demo".into(),
                    group: "vip".into(),
                }),
                balance: Some(demo_balance()),
                announcements: demo_announcements(),
                commands: Some(commands),
                events: Some(events),
                booting: false,
                logging_in: false,
                balance_loading: false,
                announcements_loading: false,
                login_error: None,
                balance_error: Some("previous failure".into()),
                announcement_error: Some("previous failure".into()),
                notice: None,
                storage_path: String::new(),
                expanded: false,
                desktop: Desktop::new(None).unwrap(),
                tray: None,
                demo: false,
                demo_refresh: None,
                last_height: 0.0,
                capture: None,
                capture_scroll: None,
                capture_right_click: None,
            },
            received_commands,
            sent_events,
        )
    }

    #[test]
    fn refresh_clears_old_content_and_requests_both_endpoints_without_duplicates() {
        let (mut app, commands, events) = app_with_channels();
        app.refresh();
        assert!(app.balance.is_none() && app.announcements.is_empty());
        assert!(app.balance_error.is_none() && app.announcement_error.is_none());
        assert!(app.balance_loading && app.announcements_loading);
        assert!(matches!(commands.try_recv().unwrap(), Command::Balance));
        assert!(matches!(
            commands.try_recv().unwrap(),
            Command::Announcements
        ));
        app.refresh();
        assert!(commands.try_recv().is_err());
        events.send(Event::Balance(Ok(demo_balance()))).unwrap();
        app.receive();
        assert!(app.balance.is_some() && !app.balance_loading);
        assert!(app.announcements.is_empty() && app.announcements_loading);
        app.refresh();
        assert!(
            commands.try_recv().is_err(),
            "must wait for announcements before refreshing again"
        );
        events
            .send(Event::Announcements(Ok(vec![Announcement {
                content: "updated announcement".into(),
                publish_date: "2026-11-25T00:00:00Z".into(),
            }])))
            .unwrap();
        app.receive();
        assert!(!app.announcements_loading);
        assert_eq!(app.announcements[0].content, "updated announcement");
    }

    #[test]
    fn failed_refresh_does_not_restore_stale_content_and_can_be_retried() {
        let (mut app, commands, events) = app_with_channels();
        app.refresh();
        events
            .send(Event::Balance(Err("balance failed".into())))
            .unwrap();
        events
            .send(Event::Announcements(Err("announcement failed".into())))
            .unwrap();
        app.receive();
        assert!(!app.balance_loading && !app.announcements_loading);
        assert!(app.balance.is_none() && app.announcements.is_empty());
        assert!(app.balance_error.is_some() && app.announcement_error.is_some());
        app.refresh();
        assert_eq!(commands.try_iter().count(), 4);
        assert!(app.balance_error.is_none() && app.announcement_error.is_none());
    }

    fn menu_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (bool, bool) {
        let mut opened = false;
        let mut button_clicked = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(380.0, 324.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.interact(
                        Rect::from_min_max(Pos2::new(20.0, 60.0), Pos2::new(360.0, 110.0)),
                        egui::Id::new("drag"),
                        Sense::drag(),
                    );
                    button_clicked = ui
                        .put(
                            Rect::from_min_size(Pos2::new(330.0, 20.0), Vec2::splat(26.0)),
                            egui::Button::new("refresh"),
                        )
                        .clicked();
                    ui.put(
                        Rect::from_min_size(Pos2::new(20.0, 115.0), Vec2::new(300.0, 32.0)),
                        egui::TextEdit::singleline(&mut String::new()),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(Rect::from_min_max(
                            Pos2::new(20.0, 180.0),
                            Pos2::new(360.0, 270.0),
                        )),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(90.0)
                                .show(ui, |ui| {
                                    for _ in 0..20 {
                                        ui.label("announcement");
                                    }
                                });
                        },
                    );
                });
                opened = window_context_menu(ctx, |ui| ui.button("close").clicked()).is_some();
            },
        );
        (opened, button_clicked)
    }

    fn click(ctx: &egui::Context, pos: Pos2, button: egui::PointerButton) -> (bool, bool) {
        menu_frame(
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        menu_frame(
            ctx,
            vec![egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn window_menu_opens_on_padding_drag_buttons_inputs_and_scrolling_content() {
        for pos in [
            Pos2::new(2.0, 2.0),
            Pos2::new(100.0, 85.0),
            Pos2::new(343.0, 33.0),
            Pos2::new(100.0, 130.0),
            Pos2::new(70.0, 205.0),
            Pos2::new(355.0, 210.0),
        ] {
            let ctx = egui::Context::default();
            menu_frame(&ctx, vec![]);
            let (opened, clicked) = click(&ctx, pos, egui::PointerButton::Secondary);
            assert!(opened, "menu missing at {pos:?}");
            assert!(!clicked, "right click activated refresh");
            // Right-click again while the menu is already open: it should relocate.
            assert!(click(&ctx, Pos2::new(10.0, 300.0), egui::PointerButton::Secondary).0);
        }
    }

    #[test]
    fn global_menu_does_not_intercept_normal_left_clicks() {
        let ctx = egui::Context::default();
        menu_frame(&ctx, vec![]);
        let (opened, clicked) = click(&ctx, Pos2::new(343.0, 33.0), egui::PointerButton::Primary);
        assert!(!opened);
        assert!(clicked);
    }
}
