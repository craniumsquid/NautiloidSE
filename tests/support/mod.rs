//! Drives the real interface without a window: feeds it input, runs frames, finds what it
//! drew, and saves pictures of it.

pub mod raster;

use eframe::egui::{
    self, Context, DroppedFile, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput,
    Rect, Shape, Vec2, ViewportEvent, ViewportId,
};
use nautiloid_save_editor::party;
use nautiloid_save_editor::save;
use nautiloid_save_editor::saves::build::{save_folder, SaveSpec};
use nautiloid_save_editor::ui::{theme, EditorApp, Options, Platform};
use raster::Renderer;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A time shortly after the newest sample save was made.
pub const NOW: u64 = 1_794_500_000;

/// What the app asked of the system, for tests to look at.
#[derive(Default)]
pub struct PlatformLog {
    /// The folder to answer the next "choose a folder" with.
    pub folder_to_pick: Option<PathBuf>,
    pub revealed: Vec<PathBuf>,
}

pub struct TestPlatform(pub Rc<RefCell<PlatformLog>>);

impl Platform for TestPlatform {
    fn pick_folder(&self, _title: &str, _start: Option<&Path>) -> Option<PathBuf> {
        self.0.borrow_mut().folder_to_pick.take()
    }
    fn reveal(&self, path: &Path) {
        self.0.borrow_mut().revealed.push(path.to_owned());
    }
}

pub struct Harness {
    pub ctx: Context,
    pub app: EditorApp,
    pub platform: Rc<RefCell<PlatformLog>>,
    renderer: Renderer,
    size: Vec2,
    time: f64,
    events: Vec<Event>,
    dropped: Vec<DroppedFile>,
    close_requested: bool,
    /// What the last frame drew.
    shapes: Vec<egui::epaint::ClippedShape>,
    pixels_per_point: f32,
    /// Viewport commands the app sent in the last frame.
    pub commands: Vec<egui::ViewportCommand>,
}

impl Harness {
    pub fn new(saves_dir: Option<PathBuf>, size: [f32; 2]) -> Harness {
        let ctx = Context::default();
        let platform = Rc::new(RefCell::new(PlatformLog::default()));
        let app = EditorApp::new(
            &ctx,
            Box::new(TestPlatform(platform.clone())),
            Options {
                saves_dir,
                detect: false,
                open: None,
                now: Some(NOW),
            },
        );
        let mut harness = Harness {
            ctx,
            app,
            platform,
            renderer: Renderer::default(),
            size: size.into(),
            time: 0.0,
            events: Vec::new(),
            dropped: Vec::new(),
            close_requested: false,
            shapes: Vec::new(),
            pixels_per_point: 1.0,
            commands: Vec::new(),
        };
        harness.settle();
        harness
    }

    /// Runs one frame with the input queued so far.
    pub fn frame(&mut self) {
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            focused: true,
            events: std::mem::take(&mut self.events),
            dropped_files: std::mem::take(&mut self.dropped),
            ..RawInput::default()
        };
        let viewport = raw.viewports.entry(ViewportId::ROOT).or_default();
        viewport.native_pixels_per_point = Some(1.0);
        if std::mem::take(&mut self.close_requested) {
            viewport.events.push(ViewportEvent::Close);
        }
        let app = &mut self.app;
        let FullOutput {
            shapes,
            textures_delta,
            pixels_per_point,
            viewport_output,
            ..
        } = self.ctx.run(raw, |ctx| app.update(ctx));
        self.renderer.update(&textures_delta);
        self.shapes = shapes;
        self.pixels_per_point = pixels_per_point;
        self.commands = viewport_output
            .get(&ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default();
        self.time += 1.0 / 60.0;
    }

    /// Runs frames until nothing is loading and the layout has stopped moving.
    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.frame();
        }
        let started = std::time::Instant::now();
        while !self.app.is_settled() && started.elapsed().as_secs() < 20 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.frame();
        }
        for _ in 0..3 {
            self.frame();
        }
    }

    /// Lets `seconds` pass without input.
    pub fn wait(&mut self, seconds: f64) {
        self.time += seconds;
        self.settle();
    }

    fn texts(&self) -> Vec<(String, Rect)> {
        fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                Shape::Text(text) => {
                    let rect = Rect::from_min_size(text.pos, text.galley.size());
                    out.push((text.galley.text().to_owned(), rect));
                }
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &self.shapes {
            let before = out.len();
            collect(&clipped.shape, &mut out);
            // Text outside its clip rectangle is not on the screen.
            let clip = clipped.clip_rect;
            let mut kept = out.split_off(before);
            kept.retain(|(_, rect)| clip.intersects(*rect));
            out.extend(kept);
        }
        out
    }

    /// Whether `text` is drawn anywhere, as a whole piece of text or part of one.
    pub fn sees(&self, text: &str) -> bool {
        self.texts().iter().any(|(t, _)| t.contains(text))
    }

    /// Where every piece of text exactly equal to `text` is, top to bottom.
    pub fn find_all(&self, text: &str) -> Vec<Rect> {
        let mut found: Vec<Rect> = self
            .texts()
            .into_iter()
            .filter(|(t, _)| t == text)
            .map(|(_, rect)| rect)
            .collect();
        found.sort_by(|a, b| {
            (a.top(), a.left())
                .partial_cmp(&(b.top(), b.left()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        found
    }

    pub fn find(&self, text: &str) -> Rect {
        *self.find_all(text).first().unwrap_or_else(|| {
            let mut seen: Vec<String> = self.texts().into_iter().map(|(t, _)| t).collect();
            seen.dedup();
            panic!("{text:?} is not on the screen. What is: {seen:?}")
        })
    }

    pub fn click_at(&mut self, pos: Pos2) {
        self.events.push(Event::PointerMoved(pos));
        self.frame();
        self.events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        self.frame();
        self.events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        self.settle();
    }

    /// Clicks the first piece of text equal to `text`.
    pub fn click(&mut self, text: &str) {
        let at = self.find(text).center();
        self.click_at(at);
    }

    /// Clicks the `n`-th piece of text equal to `text`, counting from the top.
    pub fn click_nth(&mut self, text: &str, n: usize) {
        let found = self.find_all(text);
        let rect = found
            .get(n)
            .unwrap_or_else(|| panic!("only {} {text:?} on the screen", found.len()));
        self.click_at(rect.center());
    }

    pub fn press(&mut self, key: Key, modifiers: Modifiers) {
        for pressed in [true, false] {
            self.events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        self.settle();
    }

    pub fn type_text(&mut self, text: &str) {
        self.events.push(Event::Text(text.to_owned()));
        self.settle();
    }

    pub fn drop_file(&mut self, path: &Path) {
        self.dropped.push(DroppedFile {
            path: Some(path.to_owned()),
            ..DroppedFile::default()
        });
        self.settle();
    }

    pub fn request_close(&mut self) {
        self.close_requested = true;
        self.settle();
    }

    pub fn resize(&mut self, size: [f32; 2]) {
        self.size = size.into();
        self.settle();
    }

    /// Saves a picture of the screen as it stands to `target/ui-snapshots/<name>.png`.
    pub fn snapshot(&mut self, name: &str) {
        self.frame();
        let primitives = self
            .ctx
            .tessellate(self.shapes.clone(), self.pixels_per_point);
        let (width, height) = (self.size.x as usize, self.size.y as usize);
        let background = theme::BG.to_array();
        let pixels = self.renderer.render(
            &primitives,
            width,
            height,
            self.pixels_per_point,
            [background[0], background[1], background[2]],
        );
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ui-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .expect("the picture has the size it was drawn at")
            .save(dir.join(format!("{name}.png")))
            .unwrap();
    }
}

/// A new, empty folder for one test.
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nautiloid-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A screenshot for a sample save: a sky over a dark shore, in the given colours, as a
/// real `.WebP`.
fn screenshot(sky: [u8; 3], glow: [u8; 3]) -> Vec<u8> {
    let (width, height) = (480u32, 270u32);
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        let down = y as f32 / height as f32;
        let across = (x as f32 / width as f32 - 0.62).abs();
        // A hill line, and a light low in the sky.
        let hill = 0.72 + 0.07 * ((x as f32 / 53.0).sin() + 0.5 * (x as f32 / 19.0).cos());
        if down > hill {
            let shade = (1.0 - (down - hill) * 2.0).clamp(0.3, 1.0);
            return image::Rgba([
                (14.0 * shade) as u8,
                (11.0 * shade) as u8,
                (22.0 * shade) as u8,
                255,
            ]);
        }
        let light = ((1.0 - across * 2.2).max(0.0) * (down / hill).powi(3)).clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| {
            (a as f32 * (1.0 - light) * (0.45 + 0.55 * down) + b as f32 * light) as u8
        };
        image::Rgba([
            mix(sky[0], glow[0]),
            mix(sky[1], glow[1]),
            mix(sky[2], glow[2]),
            255,
        ])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::WebP,
        )
        .unwrap();
    bytes
}

/// What to put in a sample saves folder.
pub struct Sample {
    pub leader: &'static str,
    pub name: &'static str,
    pub campaign: &'static str,
    pub save_type: u8,
    /// Seconds before [`NOW`] that the save was made.
    pub age: u64,
    pub colors: Option<([u8; 3], [u8; 3])>,
    pub mods: usize,
    /// Whether the save holds a party that can be opened and edited.
    pub with_party: bool,
}

const TAV: &str = "04f94982-eff9-ed85-72ae-f2bfb78b94b3";
const GALE: &str = "8eee0a6a-21d0-7edf-a0fd-fe044b712240";

/// A saves folder like a player's: one campaign with several saves and an Honour Mode one.
pub fn sample_saves(dir: &Path) {
    let samples = [
        Sample {
            leader: "Tav",
            name: "Shattered Sanctum - 8h 02m",
            campaign: TAV,
            save_type: 0,
            age: 900,
            colors: Some(([92, 60, 150], [255, 196, 120])),
            mods: 12,
            with_party: true,
        },
        Sample {
            leader: "Tav",
            name: "AutoSave_26",
            campaign: TAV,
            save_type: 2,
            age: 5_400,
            colors: Some(([40, 86, 130], [150, 220, 230])),
            mods: 12,
            with_party: true,
        },
        Sample {
            leader: "Tav",
            name: "QuickSave_3",
            campaign: TAV,
            save_type: 1,
            age: 20_000,
            colors: Some(([120, 50, 70], [250, 160, 110])),
            mods: 12,
            with_party: true,
        },
        Sample {
            leader: "Tav",
            name: "Waukeen's Rest - 6h 36m",
            campaign: TAV,
            save_type: 0,
            age: 180_000,
            colors: Some(([60, 100, 70], [230, 230, 150])),
            mods: 11,
            with_party: true,
        },
        Sample {
            leader: "Tav",
            name: "Ravaged Beach - 0h 38m",
            campaign: TAV,
            save_type: 0,
            age: 1_900_000,
            colors: None,
            mods: 9,
            with_party: true,
        },
        Sample {
            leader: "Gale",
            name: "HonourMode",
            campaign: GALE,
            save_type: 3,
            age: 40_000,
            colors: Some(([70, 40, 100], [240, 120, 200])),
            mods: 0,
            with_party: true,
        },
    ];
    for sample in samples {
        write_sample(dir, &sample);
    }
}

pub fn write_sample(dir: &Path, sample: &Sample) -> PathBuf {
    let mut spec = SaveSpec::new(sample.leader, sample.name);
    spec.campaign = sample.campaign.to_owned();
    spec.save_type = sample.save_type;
    spec.saved_at = NOW - sample.age;
    spec.mods = (1..=sample.mods)
        .map(|n| format!("Sample mod {n}"))
        .collect();
    spec.screenshot = sample.colors.map(|(sky, glow)| screenshot(sky, glow));
    if sample.with_party {
        spec.files = vec![
            (save::GLOBALS.to_owned(), party::build::globals_lsf()),
            (save::STORY.to_owned(), vec![7u8; 4096]),
        ];
    }
    save_folder(dir, &spec).unwrap()
}
