#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod capture;
mod desktop;
mod fonts;
mod icon;
mod model;
mod storage;
mod tray;
mod worker;

fn main() -> eframe::Result {
    let arguments: Vec<_> = std::env::args().collect();
    let capture = arguments
        .windows(2)
        .find(|pair| pair[0] == "--capture-demo")
        .map(|pair| std::path::PathBuf::from(&pair[1]));
    let demo = capture.is_some() || arguments.iter().any(|arg| arg == "--demo");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("docode-rs")
            .with_app_id("cc.docode.widget")
            .with_icon(icon::window())
            .with_inner_size([380.0, 330.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_window_level(eframe::egui::WindowLevel::AlwaysOnTop)
            .with_minimize_button(false)
            .with_resizable(false),
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "docode-rs",
        options,
        Box::new(move |cc| Ok(Box::new(app::DocodeApp::new(cc, demo, capture)))),
    )
}
