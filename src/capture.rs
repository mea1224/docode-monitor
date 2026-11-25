//! Offline render smoke test using egui's own framebuffer capture.
//! Never accesses the desktop framebuffer, accounts or external services.
use eframe::egui::{self, Context, Event, ViewportCommand};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct Capture {
    directory: PathBuf,
    pub step: usize,
    next: Instant,
    pending: bool,
    pub report: Vec<String>,
}

pub const SCENES: [&str; 8] = [
    "01-collapsed",
    "02-expanded",
    "03-scrolled",
    "04-refreshing",
    "05-pinned",
    "06-unpinned",
    "07-login",
    "08-input-context-menu",
];

impl Capture {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            step: 0,
            next: Instant::now() + Duration::from_millis(800),
            pending: false,
            report: Vec::new(),
        }
    }

    /// True after the current frame has been saved, to advance the scene.
    pub fn tick(&mut self, ctx: &Context) -> anyhow::Result<bool> {
        std::fs::create_dir_all(&self.directory)?;
        let screenshot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot.filter(|_| self.step < SCENES.len()) {
            let file = self.directory.join(format!("{}.png", SCENES[self.step]));
            let rgba: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                .collect();
            image::save_buffer(
                &file,
                &rgba,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )?;
            self.report.push(format!(
                "{}: {} x {}",
                SCENES[self.step],
                image.width(),
                image.height()
            ));
            self.step += 1;
            self.pending = false;
            self.next = Instant::now() + Duration::from_millis(200);
            std::fs::write(self.directory.join("report.txt"), self.report.join("\n"))?;
            if self.step == SCENES.len() {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return Ok(true);
        }
        if self.step < SCENES.len() && !self.pending && Instant::now() >= self.next {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            self.pending = true;
        }
        if self.pending && Instant::now().duration_since(self.next) > Duration::from_secs(10) {
            anyhow::bail!("等待渲染截图超时");
        }
        ctx.request_repaint_after(Duration::from_millis(40));
        Ok(false)
    }
}
