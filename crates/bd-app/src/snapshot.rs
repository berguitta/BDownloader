//! Headless screenshot mode (development only, `snapshot` feature).
//!
//! Renders the real UI with Slint's software renderer, without a visible
//! window, for a few seconds and saves the last frame as a PNG. Used to check
//! the UI from scripts and CI.
//!
//! ```text
//! BD_SNAPSHOT=out.png BD_SNAPSHOT_SECONDS=6 BD_SNAPSHOT_LINKS="https://…" bdownloader
//! ```
//! Optional: `BD_SNAPSHOT_PAGE` (0 downloads, 3 settings), `BD_SNAPSHOT_THEME`
//! (1 light, 2 dark), `BD_SNAPSHOT_SELECT` (row index to select),
//! `BD_SNAPSHOT_SIZE` (`1440x900`).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{PhysicalSize, PlatformError};

pub struct Snapshot {
    pub output: PathBuf,
    pub seconds: f64,
    pub links: Option<String>,
    pub page: Option<i32>,
    pub theme: Option<i32>,
    pub select: Option<i32>,
}

impl Snapshot {
    pub fn from_env() -> Option<Self> {
        let output = PathBuf::from(std::env::var_os("BD_SNAPSHOT")?);
        let num = |key: &str| std::env::var(key).ok().and_then(|v| v.parse::<i32>().ok());
        Some(Snapshot {
            output,
            seconds: std::env::var("BD_SNAPSHOT_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0),
            links: std::env::var("BD_SNAPSHOT_LINKS").ok(),
            page: num("BD_SNAPSHOT_PAGE"),
            theme: num("BD_SNAPSHOT_THEME"),
            select: num("BD_SNAPSHOT_SELECT"),
        })
    }

    /// Must run before the first window is created.
    pub fn install(&self) -> Result<(), PlatformError> {
        let (width, height) = std::env::var("BD_SNAPSHOT_SIZE")
            .ok()
            .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
            .unwrap_or((1440u32, 900u32));
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        window.set_size(PhysicalSize::new(width, height));
        slint::platform::set_platform(Box::new(SnapshotPlatform {
            window,
            start: Instant::now(),
            output: self.output.clone(),
            seconds: self.seconds,
            size: (width, height),
        }))
        .map_err(|e| PlatformError::Other(e.to_string()))
    }
}

struct SnapshotPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
    output: PathBuf,
    seconds: f64,
    size: (u32, u32),
}

impl Platform for SnapshotPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let mut buffer = vec![PremultipliedRgbaColor::default(); width * height];
        while self.start.elapsed().as_secs_f64() < self.seconds {
            slint::platform::update_timers_and_animations();
            self.window.request_redraw();
            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut buffer, width);
            });
            std::thread::sleep(Duration::from_millis(30));
        }
        save_png(&self.output, &buffer, self.size).map_err(|e| PlatformError::Other(e.to_string()))
    }
}

fn save_png(path: &PathBuf, pixels: &[PremultipliedRgbaColor], (width, height): (u32, u32)) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    // The window background is opaque, so premultiplied RGB is plain RGB.
    let data: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue]).collect();
    writer.write_image_data(&data).map_err(std::io::Error::other)
}
