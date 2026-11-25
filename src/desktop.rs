use anyhow::{Result, bail};
#[cfg(any(not(windows), test))]
use eframe::egui::WindowLevel;
use eframe::egui::{self, Context, ViewportCommand};
use raw_window_handle::RawWindowHandle;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_app_kit::{NSView, NSWindowCollectionBehavior};

    #[derive(Default)]
    pub(super) struct State {
        original: Option<NSWindowCollectionBehavior>,
        original_level: Option<isize>,
    }

    impl State {
        pub(super) fn set_pinned(&mut self, handle: RawWindowHandle, pinned: bool) -> Result<()> {
            let RawWindowHandle::AppKit(handle) = handle else {
                bail!("不是 macOS 原生窗口");
            };
            // Called on the eframe main thread, while its NSView is alive.
            let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
            let window = view
                .window()
                .ok_or_else(|| anyhow::anyhow!("无法获取 macOS 窗口"))?;
            if pinned {
                self.original = Some(window.collectionBehavior());
                self.original_level = Some(window.level());
                window.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::Stationary
                        | NSWindowCollectionBehavior::IgnoresCycle,
                );
                window.setLevel(-1);
            } else if let Some(original) = self.original.take() {
                window.setCollectionBehavior(original);
                if let Some(level) = self.original_level.take() {
                    window.setLevel(level);
                }
            }
            Ok(())
        }
    }
}

/// A desktop widget while pinned; a movable floating window otherwise.
pub struct Desktop {
    handle: Option<RawWindowHandle>,
    pub pinned: bool,
    #[cfg(target_os = "macos")]
    mac: macos::State,
}

impl Desktop {
    pub fn new(handle: Option<RawWindowHandle>) -> Result<Self> {
        #[cfg(windows)]
        if let Some(RawWindowHandle::Win32(window)) = handle {
            windows::install(window.hwnd.get() as _)?;
        }
        Ok(Self {
            handle,
            pinned: false,
            #[cfg(target_os = "macos")]
            mac: macos::State::default(),
        })
    }

    pub fn set_pinned(&mut self, pinned: bool, ctx: &Context) -> Result<()> {
        if pinned == self.pinned {
            return Ok(());
        }
        let handle = self
            .handle
            .ok_or_else(|| anyhow::anyhow!("无法获取原生窗口句柄"))?;
        if pinned && matches!(handle, RawWindowHandle::Wayland(_)) {
            bail!(
                "当前 Wayland 窗口后端不支持设置窗口层级。请在 X11 会话运行，或在 XWayland 可用时使用 WINIT_UNIX_BACKEND=x11。"
            );
        }
        #[cfg(windows)]
        {
            let RawWindowHandle::Win32(handle) = handle else {
                bail!("不是 Windows 原生窗口");
            };
            windows::set_pinned(handle.hwnd.get() as _, pinned)?;
        }
        #[cfg(target_os = "macos")]
        self.mac.set_pinned(handle, pinned)?;
        // On Windows the native controller owns the level. Repeating it through
        // winit also calls ShowWindow and rebuilds the frame/styles, causing
        // unnecessary activation and repaint during a pin transition.
        #[cfg(not(windows))]
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if pinned {
            WindowLevel::AlwaysOnBottom
        } else {
            WindowLevel::AlwaysOnTop
        }));
        if !pinned {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        self.pinned = pinned;
        Ok(())
    }

    pub fn focus_on_pointer_press(&self, ctx: &Context) {
        let viewport = ctx.viewport_rect();
        if !self.pinned
            && ctx.input(|input| {
                input.pointer.any_pressed()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| viewport.contains(pos))
            })
        {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    pub fn drag_area(&self, ui: &mut egui::Ui, rect: egui::Rect, id: &'static str) {
        if self.pinned {
            return;
        }
        let response = ui.interact(rect, ui.id().with(id), egui::Sense::drag());
        if response.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(RawWindowHandle::Win32(handle)) = self.handle {
            windows::uninstall(handle.hwnd.get() as _);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer_frame(
        desktop: &Desktop,
        ctx: &Context,
        events: Vec<egui::Event>,
    ) -> Vec<ViewportCommand> {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(380.0, 324.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    desktop.drag_area(ui, ui.max_rect(), "test-drag");
                });
                desktop.focus_on_pointer_press(ctx);
            },
        );
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .clone()
    }

    #[test]
    fn clicking_and_dragging_raise_only_unpinned_windows() {
        let mut desktop = Desktop::new(None).unwrap();
        for pinned in [false, true, false] {
            desktop.pinned = pinned;
            let ctx = Context::default();
            assert!(pointer_frame(&desktop, &ctx, vec![]).is_empty());
            let start = egui::pos2(50.0, 50.0);
            let mut commands = pointer_frame(
                &desktop,
                &ctx,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            commands.extend(pointer_frame(
                &desktop,
                &ctx,
                vec![egui::Event::PointerMoved(egui::pos2(100.0, 80.0))],
            ));
            assert_eq!(commands.contains(&ViewportCommand::Focus), !pinned);
            assert_eq!(commands.contains(&ViewportCommand::StartDrag), !pinned);
            assert!(!commands.contains(&ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop)));
        }
    }
}

#[cfg(windows)]
mod windows {
    use anyhow::{Result, bail};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, SetLastError, WPARAM},
        Graphics::Dwm::{
            DWMWA_EXCLUDED_FROM_PEEK, DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
        },
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    };

    const SUBCLASS_ID: usize = 0x444F434F;
    const DESKTOP_TIMER: usize = 0x444F434F;
    const OWNER_PROPERTY: windows_sys::core::PCWSTR = windows_sys::w!("DoCode.OriginalOwner");
    // Non-null values distinguish installed/unpinned (1) and pinned (2).
    const STATE_PROPERTY: windows_sys::core::PCWSTR = windows_sys::w!("DoCode.WindowState");

    pub(super) fn install(hwnd: HWND) -> Result<()> {
        install_with_owner(hwnd, desktop_owner())
    }

    fn install_with_owner(hwnd: HWND, owner: HWND) -> Result<()> {
        unsafe {
            let previous = GetWindow(hwnd, GW_OWNER);
            if !previous.is_null() && SetPropW(hwnd, OWNER_PROPERTY, previous) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if SetPropW(hwnd, STATE_PROPERTY, 1usize as _) == 0 {
                RemovePropW(hwnd, OWNER_PROPERTY);
                return Err(std::io::Error::last_os_error().into());
            }
            if SetWindowSubclass(hwnd, Some(widget_window), SUBCLASS_ID, 0) == 0 {
                RemovePropW(hwnd, OWNER_PROPERTY);
                RemovePropW(hwnd, STATE_PROPERTY);
                bail!("安装窗口消息处理器失败");
            }
            // Keep the desktop owner for the entire lifetime, including the
            // movable/topmost state. Never detach on unpin or hide/restore to
            // repair visibility after Show Desktop has already happened.
            let result = (|| {
                if !owner.is_null() {
                    set_owner(hwnd, owner)?;
                }
                set_level(hwnd, false)?;
                if SetTimer(hwnd, DESKTOP_TIMER, 1000, None) == 0 {
                    bail!("创建桌面宿主检测定时器失败");
                }
                Ok(())
            })();
            if result.is_err() {
                uninstall(hwnd);
                return result;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style & !(WS_EX_APPWINDOW as isize)) | WS_EX_TOOLWINDOW as isize,
            );
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_MINIMIZEBOX as isize));
            exclude_from_desktop_effects(hwnd);
        }
        Ok(())
    }

    fn exclude_from_desktop_effects(hwnd: HWND) {
        unsafe {
            // Peek is compositor-driven: blocking SW_HIDE/SC_MINIMIZE alone
            // cannot stop its fade. Opt this widget out before it is shown.
            let enabled: i32 = 1;
            for attribute in [DWMWA_EXCLUDED_FROM_PEEK, DWMWA_TRANSITIONS_FORCEDISABLED] {
                // DWM can be unavailable (e.g. a headless session). The native
                // hide/minimize guards remain active; retry on composition changes.
                DwmSetWindowAttribute(
                    hwnd,
                    attribute as u32,
                    (&enabled as *const i32).cast(),
                    std::mem::size_of_val(&enabled) as u32,
                );
            }
        }
    }

    fn desktop_owner() -> HWND {
        unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> i32 {
            unsafe {
                let mut class = [0u16; 64];
                let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
                if String::from_utf16_lossy(&class[..len.max(0) as usize]) == "SHELLDLL_DefView" {
                    *(data as *mut HWND) = GetAncestor(hwnd, GA_ROOT);
                    return 0;
                }
                1
            }
        }
        unsafe extern "system" fn find_host(hwnd: HWND, data: LPARAM) -> i32 {
            unsafe {
                EnumChildWindows(hwnd, Some(find), data);
                (*(data as *const HWND)).is_null() as i32
            }
        }
        let mut owner: HWND = std::ptr::null_mut();
        unsafe {
            // Windows 11 can nest the icon view inside another shell window.
            let shell = GetShellWindow();
            if !shell.is_null() {
                EnumWindows(Some(find_host), &mut owner as *mut HWND as LPARAM);
                if owner.is_null() {
                    owner = shell;
                }
            }
        }
        owner
    }

    fn set_owner(hwnd: HWND, owner: HWND) -> Result<()> {
        unsafe {
            SetLastError(0);
            if SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize) == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn set_pinned(hwnd: HWND, pinned: bool) -> Result<()> {
        // Unpin only changes the level; keep the current desktop association.
        // The existing timer handles a shell restart in either mode.
        let owner = if pinned {
            desktop_owner()
        } else {
            std::ptr::null_mut()
        };
        if pinned && owner.is_null() {
            bail!("未找到 Windows 桌面，请等待资源管理器启动后重试");
        }
        set_pinned_with_owner(hwnd, pinned, owner)
    }

    fn set_pinned_with_owner(hwnd: HWND, pinned: bool, owner: HWND) -> Result<()> {
        unsafe {
            let previous = GetPropW(hwnd, STATE_PROPERTY) as usize;
            if previous == 0 {
                bail!("窗口消息处理器未初始化");
            }
            if !owner.is_null() && GetWindow(hwnd, GW_OWNER) != owner {
                set_owner(hwnd, owner)?;
            }
            if previous == pinned as usize + 1 {
                return Ok(());
            }
            if SetPropW(hwnd, STATE_PROPERTY, (pinned as usize + 1) as _) == 0 {
                bail!("更新窗口固定状态失败");
            }
            if let Err(error) = set_level(hwnd, pinned) {
                SetPropW(hwnd, STATE_PROPERTY, previous as _);
                return Err(error);
            }
        }
        Ok(())
    }

    fn set_level(hwnd: HWND, pinned: bool) -> Result<()> {
        unsafe {
            if SetWindowPos(
                hwnd,
                if pinned { HWND_BOTTOM } else { HWND_TOPMOST },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }

    pub(super) fn uninstall(hwnd: HWND) {
        unsafe {
            if GetPropW(hwnd, STATE_PROPERTY).is_null() {
                return;
            }
            let previous = GetPropW(hwnd, OWNER_PROPERTY);
            remove_hook(hwnd);
            let _ = set_owner(hwnd, previous);
        }
    }

    fn remove_hook(hwnd: HWND) {
        unsafe {
            KillTimer(hwnd, DESKTOP_TIMER);
            RemovePropW(hwnd, OWNER_PROPERTY);
            RemovePropW(hwnd, STATE_PROPERTY);
            RemoveWindowSubclass(hwnd, Some(widget_window), SUBCLASS_ID);
        }
    }

    unsafe extern "system" fn widget_window(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        unsafe {
            let pinned = GetPropW(hwnd, STATE_PROPERTY) as usize == 2;
            if message == WM_STYLECHANGING && lparam != 0 {
                let style = &mut *(lparam as *mut STYLESTRUCT);
                if wparam as i32 == GWL_EXSTYLE {
                    // Preserve native state when winit rebuilds cached styles.
                    style.styleNew = (style.styleNew & !(WS_EX_APPWINDOW | WS_EX_TOPMOST))
                        | WS_EX_TOOLWINDOW
                        | if pinned { 0 } else { WS_EX_TOPMOST };
                } else if wparam as i32 == GWL_STYLE {
                    style.styleNew &= !(WS_MINIMIZE | WS_MINIMIZEBOX);
                }
            } else if message == WM_WINDOWPOSCHANGING && lparam != 0 {
                let position = &mut *(lparam as *mut WINDOWPOS);
                position.flags &= !SWP_HIDEWINDOW;
                if pinned {
                    // Do not turn resize/no-op messages into z-order changes.
                    // In particular, do not fight the shell's owner/owned
                    // ordering while it brings the desktop forward.
                    position.flags |= SWP_NOACTIVATE | SWP_NOMOVE;
                    if position.flags & SWP_NOZORDER == 0 {
                        position.hwndInsertAfter = HWND_BOTTOM;
                    }
                } else if position.flags & SWP_NOZORDER == 0 {
                    position.hwndInsertAfter = HWND_TOPMOST;
                    position.flags |= SWP_NOOWNERZORDER;
                }
            } else if message == WM_SYSCOMMAND && (wparam as u32 & 0xfff0) == SC_MINIMIZE {
                return 0;
            } else if message == WM_SHOWWINDOW && wparam == 0 && lparam != 0 {
                // DefWindowProc otherwise hides owned popups on owner/shell
                // notifications. Stop the hide before any frame is lost.
                return 0;
            } else if message == WM_MOUSEACTIVATE && pinned {
                return MA_NOACTIVATE as LRESULT;
            } else if message == WM_ERASEBKGND {
                // The transparent OpenGL framebuffer paints the whole client.
                return 1;
            } else if message == WM_DWMCOMPOSITIONCHANGED {
                exclude_from_desktop_effects(hwnd);
            } else if message == WM_TIMER && wparam == DESKTOP_TIMER {
                // Explorer can recreate its desktop (restart / monitor change).
                // Reattach ownership without moving or activating the widget.
                let owner = desktop_owner();
                if !owner.is_null()
                    && GetWindow(hwnd, GW_OWNER) != owner
                    && set_owner(hwnd, owner).is_ok()
                {
                    let _ = set_level(hwnd, pinned);
                }
                return 0;
            } else if message == WM_NCDESTROY {
                remove_hook(hwnd);
            }
            DefSubclassProc(hwnd, message, wparam, lparam)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ptr::null_mut;
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::w;

        // Z-order belongs to the shared desktop; these checks must not race.
        static WINDOW_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn bounds(hwnd: HWND) -> (i32, i32, i32, i32) {
            let mut rect = RECT::default();
            assert_ne!(unsafe { GetWindowRect(hwnd, &mut rect) }, 0);
            (
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            )
        }

        struct TestWindow(HWND);
        impl TestWindow {
            fn new() -> Self {
                // Hidden, unowned test windows: no user windows are manipulated.
                let hwnd = unsafe {
                    CreateWindowExW(
                        0,
                        w!("STATIC"),
                        w!("docode-layer-test"),
                        WS_POPUP,
                        0,
                        0,
                        120,
                        80,
                        null_mut(),
                        null_mut(),
                        null_mut(),
                        null_mut(),
                    )
                };
                assert!(!hwnd.is_null(), "{}", std::io::Error::last_os_error());
                Self(hwnd)
            }
        }
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }

        fn below(window: HWND, other: HWND) -> bool {
            unsafe {
                let mut current = GetWindow(other, GW_HWNDNEXT);
                while !current.is_null() {
                    if current == window {
                        return true;
                    }
                    current = GetWindow(current, GW_HWNDNEXT);
                }
            }
            false
        }

        #[derive(Default, Debug)]
        struct WindowEvents {
            hidden: usize,
            minimized: usize,
            reordered: usize,
        }

        unsafe extern "system" fn observe_changes(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _id: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                let events = &mut *(data as *mut WindowEvents);
                if message == WM_WINDOWPOSCHANGED {
                    let position = &*(lparam as *const WINDOWPOS);
                    events.hidden += usize::from(position.flags & SWP_HIDEWINDOW != 0);
                    events.reordered += usize::from(position.flags & SWP_NOZORDER == 0);
                } else if message == WM_SIZE && wparam == SIZE_MINIMIZED as usize {
                    events.minimized += 1;
                }
                DefSubclassProc(hwnd, message, wparam, lparam)
            }
        }

        #[test]
        fn both_modes_reject_shell_hide_without_a_hidden_or_minimized_frame() {
            let _guard = WINDOW_TEST_LOCK.lock().unwrap();
            let mut events = WindowEvents::default();
            let shell = TestWindow::new();
            let widget = TestWindow::new();
            unsafe {
                // Visible but off-screen; never activate or touch user windows.
                SetWindowPos(
                    widget.0,
                    HWND_TOP,
                    -30000,
                    -30000,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
                install_with_owner(widget.0, shell.0).unwrap();
                ShowWindow(widget.0, SW_SHOWNOACTIVATE);
                assert_ne!(IsWindowVisible(widget.0), 0);
                assert_ne!(
                    SetWindowSubclass(
                        widget.0,
                        Some(observe_changes),
                        SUBCLASS_ID + 10,
                        &mut events as *mut _ as usize
                    ),
                    0
                );
                for pinned in [false, true, false, true] {
                    set_pinned_with_owner(widget.0, pinned, shell.0).unwrap();
                    let before = bounds(widget.0);
                    for _ in 0..8 {
                        SendMessageW(widget.0, WM_SYSCOMMAND, SC_MINIMIZE as usize, 0);
                        SendMessageW(widget.0, WM_SHOWWINDOW, 0, SW_PARENTCLOSING as isize);
                        SendMessageW(widget.0, WM_SHOWWINDOW, 0, SW_OTHERZOOM as isize);
                        SetWindowPos(
                            widget.0,
                            std::ptr::null_mut(),
                            0,
                            0,
                            0,
                            0,
                            SWP_HIDEWINDOW
                                | SWP_NOMOVE
                                | SWP_NOSIZE
                                | SWP_NOZORDER
                                | SWP_NOACTIVATE,
                        );
                        ShowWindow(widget.0, SW_HIDE);
                        assert_ne!(IsWindowVisible(widget.0), 0, "hidden in pinned={pinned}");
                        assert_eq!(IsIconic(widget.0), 0);
                        assert_eq!(bounds(widget.0), before);
                    }
                }
                assert_eq!(events.hidden, 0, "transient hide: {events:?}");
                assert_eq!(events.minimized, 0, "transient minimize: {events:?}");
                RemoveWindowSubclass(widget.0, Some(observe_changes), SUBCLASS_ID + 10);
            }
        }

        #[test]
        fn resizing_and_noop_messages_do_not_change_z_order() {
            let _guard = WINDOW_TEST_LOCK.lock().unwrap();
            let mut events = WindowEvents::default();
            let shell = TestWindow::new();
            let widget = TestWindow::new();
            install_with_owner(widget.0, shell.0).unwrap();
            unsafe {
                SetWindowSubclass(
                    widget.0,
                    Some(observe_changes),
                    SUBCLASS_ID + 10,
                    &mut events as *mut _ as usize,
                );
                for pinned in [false, true, false] {
                    set_pinned_with_owner(widget.0, pinned, shell.0).unwrap();
                    events.reordered = 0;
                    for height in [200, 80, 200, 80] {
                        SetWindowPos(
                            widget.0,
                            std::ptr::null_mut(),
                            0,
                            0,
                            120,
                            height,
                            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        assert_eq!(bounds(widget.0).3, height);
                    }
                    set_pinned_with_owner(widget.0, pinned, shell.0).unwrap();
                    assert_eq!(
                        events.reordered, 0,
                        "resize/no-op changed z-order: {events:?}"
                    );
                }
                RemoveWindowSubclass(widget.0, Some(observe_changes), SUBCLASS_ID + 10);
            }
        }

        #[test]
        fn pinned_window_stays_above_desktop_below_peer_and_rejects_move_and_hide() {
            let _guard = WINDOW_TEST_LOCK.lock().unwrap();
            let widget = TestWindow::new();
            let peer = TestWindow::new();
            let shell = TestWindow::new();
            install_with_owner(widget.0, shell.0).unwrap();
            unsafe {
                let original_style = GetWindowLongPtrW(widget.0, GWL_STYLE);
                let original_bounds = bounds(widget.0);
                SetWindowPos(
                    shell.0,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                set_pinned_with_owner(widget.0, true, shell.0).unwrap();
                assert_eq!(GetWindow(widget.0, GW_OWNER), shell.0);
                assert!(below(shell.0, widget.0));
                // Model the shell raising the desktop during Show Desktop.
                SetWindowPos(
                    shell.0,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                assert!(
                    below(shell.0, widget.0),
                    "desktop must never cover its widget"
                );
                SetWindowPos(
                    shell.0,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                assert_eq!(GetWindowLongPtrW(widget.0, GWL_STYLE), original_style);
                assert!(below(widget.0, peer.0));
                for flags in [SWP_NOMOVE | SWP_NOSIZE, SWP_NOSIZE | SWP_NOZORDER] {
                    assert_ne!(
                        SetWindowPos(widget.0, HWND_TOP, 15, 20, 0, 0, flags | SWP_NOACTIVATE),
                        0
                    );
                    assert!(below(widget.0, peer.0));
                    assert_eq!(bounds(widget.0), original_bounds, "pinned window moved");
                }
                assert_ne!(
                    SetWindowPos(widget.0, HWND_TOP, 30, 40, 120, 200, SWP_NOACTIVATE),
                    0
                );
                assert_eq!(
                    bounds(widget.0),
                    (original_bounds.0, original_bounds.1, 120, 200)
                );
                assert!(below(widget.0, peer.0));
                set_pinned(widget.0, false).unwrap();
                assert_eq!(GetWindow(widget.0, GW_OWNER), shell.0);
                assert_ne!(
                    GetWindowLongPtrW(widget.0, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                    0
                );
                // Raising another ordinary window must not cover the unpinned widget.
                assert_ne!(
                    SetWindowPos(
                        peer.0,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
                    ),
                    0
                );
                assert!(below(peer.0, widget.0));
                assert_ne!(
                    SetWindowPos(
                        widget.0,
                        HWND_TOP,
                        45,
                        60,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE
                    ),
                    0
                );
                assert!(
                    below(peer.0, widget.0),
                    "unpin must keep the widget above ordinary windows"
                );
                assert_eq!(GetWindow(widget.0, GW_OWNER), shell.0);
                assert_eq!(bounds(widget.0), (45, 60, 120, 200));
                assert_eq!(GetWindowLongPtrW(widget.0, GWL_STYLE), original_style);
                set_pinned_with_owner(widget.0, true, shell.0).unwrap();
                assert_eq!(
                    GetWindowLongPtrW(widget.0, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                    0
                );
                assert!(below(widget.0, peer.0));
                let mut position = WINDOWPOS {
                    hwnd: widget.0,
                    flags: SWP_HIDEWINDOW,
                    ..Default::default()
                };
                SendMessageW(
                    widget.0,
                    WM_WINDOWPOSCHANGING,
                    0,
                    &mut position as *mut _ as LPARAM,
                );
                assert_eq!(position.flags & SWP_HIDEWINDOW, 0);
                SendMessageW(widget.0, WM_SYSCOMMAND, SC_MINIMIZE as usize, 0);
                assert_eq!(IsIconic(widget.0), 0);
            }
        }

        #[test]
        fn taskbar_style_and_desktop_owner_survive_pin_cycles() {
            let _guard = WINDOW_TEST_LOCK.lock().unwrap();
            let original_owner = TestWindow::new();
            let shell = TestWindow::new();
            let widget = TestWindow::new();
            set_owner(widget.0, original_owner.0).unwrap();
            install_with_owner(widget.0, shell.0).unwrap();
            set_pinned_with_owner(widget.0, true, shell.0).unwrap();
            set_pinned(widget.0, false).unwrap();
            unsafe {
                // Simulate winit rebuilding its extended styles after unpin.
                SetWindowLongPtrW(widget.0, GWL_EXSTYLE, WS_EX_APPWINDOW as isize);
                let style = GetWindowLongPtrW(widget.0, GWL_EXSTYLE);
                assert_eq!(style & WS_EX_APPWINDOW as isize, 0);
                assert_ne!(style & WS_EX_TOOLWINDOW as isize, 0);
                assert_ne!(style & WS_EX_TOPMOST as isize, 0);
                assert_eq!(GetWindow(widget.0, GW_OWNER), shell.0);
                uninstall(widget.0);
                assert_eq!(GetWindow(widget.0, GW_OWNER), original_owner.0);
            }
        }

        #[test]
        fn unpin_keeps_desktop_owner_without_redundant_framework_commands() {
            let _guard = WINDOW_TEST_LOCK.lock().unwrap();
            use eframe::egui::{Context, ViewportCommand, ViewportId};
            use raw_window_handle::{RawWindowHandle, Win32WindowHandle};
            let widget = TestWindow::new();
            let handle =
                Win32WindowHandle::new(std::num::NonZeroIsize::new(widget.0 as isize).unwrap());
            let mut desktop =
                crate::desktop::Desktop::new(Some(RawWindowHandle::Win32(handle))).unwrap();
            assert!(!desktop.pinned);
            assert_ne!(
                unsafe { GetWindowLongPtrW(widget.0, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize,
                0
            );
            let ctx = Context::default();
            let shell = TestWindow::new();
            set_pinned_with_owner(widget.0, true, shell.0).unwrap();
            desktop.pinned = true;
            let output = ctx.run(Default::default(), |ctx| {
                desktop.set_pinned(false, ctx).unwrap()
            });
            let commands = &output.viewport_output[&ViewportId::ROOT].commands;
            assert_eq!(commands, &[ViewportCommand::Focus]);
            assert_ne!(
                unsafe { GetWindowLongPtrW(widget.0, GWL_EXSTYLE) } & WS_EX_TOPMOST as isize,
                0
            );
            assert_eq!(unsafe { GetWindow(widget.0, GW_OWNER) }, shell.0);
        }
    }
}
