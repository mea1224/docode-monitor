use image::{DynamicImage, RgbaImage, imageops};
use std::{env, fs, path::PathBuf};

fn scaled_icon(source: &DynamicImage, size: u32) -> RgbaImage {
    let resized = source
        .resize(size, size, imageops::FilterType::Lanczos3)
        .to_rgba8();
    let mut canvas = RgbaImage::new(size, size);
    imageops::overlay(
        &mut canvas,
        &resized,
        ((size - resized.width()) / 2) as i64,
        ((size - resized.height()) / 2) as i64,
    );
    canvas
}

fn main() {
    println!("cargo:rerun-if-changed=icon.webp");
    println!("cargo:rerun-if-changed=build.rs");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let source = image::open("icon.webp").expect("read project icon.webp");
    scaled_icon(&source, 256)
        .save(output.join("icon.png"))
        .expect("write window icon");
    scaled_icon(&source, 32)
        .save(output.join("tray-icon.png"))
        .expect("write tray icon");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
        for size in [16, 20, 24, 32, 48, 64, 128, 256] {
            let icon =
                ico::IconImage::from_rgba_data(size, size, scaled_icon(&source, size).into_raw());
            directory.add_entry(ico::IconDirEntry::encode(&icon).expect("encode executable icon"));
        }
        let icon_path = output.join("icon.ico");
        directory
            .write(fs::File::create(&icon_path).expect("create ICO"))
            .expect("write ICO");
        let resource = output.join("icon.rc");
        let resource_path = icon_path
            .to_str()
            .expect("icon path must be Unicode")
            .replace('\\', "/");
        fs::write(&resource, format!("1 ICON \"{resource_path}\"\n")).expect("write icon resource");
        embed_resource::compile(&resource, embed_resource::NONE)
            .manifest_required()
            .expect("embed executable icon");
    }
}
