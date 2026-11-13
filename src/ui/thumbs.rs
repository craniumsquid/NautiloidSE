//! Save screenshots, decoded off the interface thread and kept as textures.

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

/// Thumbnails are scaled down to this width; cards are never drawn wider.
const MAX_WIDTH: u32 = 560;

enum State {
    Loading,
    Ready(TextureHandle),
    /// Missing, or not a picture this can read. The card shows a placeholder.
    Failed,
}

pub struct Thumbnails {
    states: HashMap<PathBuf, State>,
    requests: Sender<PathBuf>,
    results: Receiver<(PathBuf, Option<ColorImage>)>,
    loading: usize,
}

fn decode(path: &Path) -> Option<ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP).ok()?;
    let image = if image.width() > MAX_WIDTH {
        let height = (image.height() as u64 * MAX_WIDTH as u64 / image.width() as u64).max(1);
        image.thumbnail(MAX_WIDTH, height as u32)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

impl Thumbnails {
    pub fn new(ctx: &Context) -> Thumbnails {
        let (requests, incoming) = channel::<PathBuf>();
        let (outgoing, results) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Ends when the app drops its end of either channel.
            while let Ok(path) = incoming.recv() {
                let image = decode(&path);
                if outgoing.send((path, image)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Thumbnails {
            states: HashMap::new(),
            requests,
            results,
            loading: 0,
        }
    }

    /// The texture for the screenshot at `path`, once it is ready. Asking starts the load.
    pub fn get(&mut self, path: &Path) -> Option<&TextureHandle> {
        if !self.states.contains_key(path) {
            let state = match self.requests.send(path.to_owned()) {
                Ok(()) => {
                    self.loading += 1;
                    State::Loading
                }
                Err(_) => State::Failed,
            };
            self.states.insert(path.to_owned(), state);
        }
        match self.states.get(path) {
            Some(State::Ready(texture)) => Some(texture),
            _ => None,
        }
    }

    /// Takes in whatever has finished decoding. Called once a frame.
    pub fn poll(&mut self, ctx: &Context) {
        while let Ok((path, image)) = self.results.try_recv() {
            self.loading = self.loading.saturating_sub(1);
            let state = match image {
                Some(image) => State::Ready(ctx.load_texture(
                    path.to_string_lossy(),
                    image,
                    TextureOptions::LINEAR,
                )),
                None => State::Failed,
            };
            self.states.insert(path, state);
        }
    }

    /// Whether every screenshot asked for has been decoded.
    pub fn is_idle(&self) -> bool {
        self.loading == 0
    }

    /// Drops every texture, for when the saves folder changes or is read again.
    pub fn clear(&mut self) {
        self.states
            .retain(|_, state| matches!(state, State::Loading));
    }
}
