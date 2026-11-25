use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};

pub fn install(ctx: &Context) -> bool {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("DOCODE_FONT") {
        candidates.push(std::path::PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        for font in ["msyh.ttc", "msyh.ttf", "simhei.ttf", "simsun.ttc"] {
            candidates.push(std::path::PathBuf::from(&root).join("Fonts").join(font));
        }
    }
    #[cfg(target_os = "macos")]
    for path in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
    ] {
        candidates.push(path.into());
    }
    #[cfg(target_os = "linux")]
    for path in [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    ] {
        candidates.push(path.into());
    }
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = FontDefinitions::default();
            fonts
                .font_data
                .insert("chinese".into(), FontData::from_owned(bytes).into());
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("chinese".into());
            }
            ctx.set_fonts(fonts);
            return true;
        }
    }
    false
}
