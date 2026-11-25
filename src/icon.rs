//! Both assets are generated from the root icon.webp at build time.
fn decode(bytes: &[u8]) -> eframe::egui::IconData {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("valid build-generated icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

pub fn window() -> eframe::egui::IconData {
    decode(include_bytes!(concat!(env!("OUT_DIR"), "/icon.png")))
}

#[cfg(any(windows, target_os = "macos"))]
pub fn tray() -> eframe::egui::IconData {
    decode(include_bytes!(concat!(env!("OUT_DIR"), "/tray-icon.png")))
}
