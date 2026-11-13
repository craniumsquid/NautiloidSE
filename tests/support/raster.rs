//! Draws what egui asks for into a picture, on the processor.
//!
//! egui hands a frame over as triangles: positions, colours and texture coordinates. Its
//! renderers send those to a graphics card. The tests have none, so this fills the triangles
//! itself. It is slow and does no more than the interface needs, which is enough to save a
//! picture of every screen for people to look at.

use eframe::egui::epaint::{ClippedPrimitive, ImageData, Primitive};
use eframe::egui::{Color32, TextureId, TexturesDelta};
use std::collections::HashMap;

struct Texture {
    width: usize,
    height: usize,
    /// Premultiplied, as egui's colours are.
    pixels: Vec<Color32>,
}

impl Texture {
    /// The colour at texture coordinates `(u, v)`, blended from the four nearest texels.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let texel = |x: usize, y: usize| self.pixels[y * self.width + x].to_array();
        let (a, b, c, d) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] as f32 * (1.0 - fx) + b[i] as f32 * fx;
            let bottom = c[i] as f32 * (1.0 - fx) + d[i] as f32 * fx;
            out[i] = top * (1.0 - fy) + bottom * fy;
        }
        out
    }
}

#[derive(Default)]
pub struct Renderer {
    textures: HashMap<TextureId, Texture>,
}

impl Renderer {
    /// Takes in the textures a frame created, changed or dropped.
    pub fn update(&mut self, delta: &TexturesDelta) {
        for (id, change) in &delta.set {
            let (size, pixels): ([usize; 2], Vec<Color32>) = match &change.image {
                ImageData::Color(image) => (image.size, image.pixels.clone()),
                ImageData::Font(image) => (image.size, image.srgba_pixels(None).collect()),
            };
            match change.pos {
                // Part of an existing texture, as when the font atlas gains glyphs.
                Some([left, top]) => {
                    if let Some(texture) = self.textures.get_mut(id) {
                        for row in 0..size[1] {
                            let at = (top + row) * texture.width + left;
                            texture.pixels[at..at + size[0]]
                                .copy_from_slice(&pixels[row * size[0]..(row + 1) * size[0]]);
                        }
                    }
                }
                None => {
                    self.textures.insert(
                        *id,
                        Texture {
                            width: size[0],
                            height: size[1],
                            pixels,
                        },
                    );
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Draws `primitives` over a `background` and returns the picture as RGBA bytes.
    pub fn render(
        &self,
        primitives: &[ClippedPrimitive],
        width: usize,
        height: usize,
        pixels_per_point: f32,
        background: [u8; 3],
    ) -> Vec<u8> {
        let mut frame = vec![
            [
                background[0] as f32,
                background[1] as f32,
                background[2] as f32
            ];
            width * height
        ];
        for clipped in primitives {
            let Primitive::Mesh(mesh) = &clipped.primitive else {
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            let clip = clipped.clip_rect;
            let clip_left = ((clip.min.x * pixels_per_point).round().max(0.0) as usize).min(width);
            let clip_top = ((clip.min.y * pixels_per_point).round().max(0.0) as usize).min(height);
            let clip_right = ((clip.max.x * pixels_per_point).round().max(0.0) as usize).min(width);
            let clip_bottom =
                ((clip.max.y * pixels_per_point).round().max(0.0) as usize).min(height);

            for triangle in mesh.indices.chunks_exact(3) {
                let v = [
                    &mesh.vertices[triangle[0] as usize],
                    &mesh.vertices[triangle[1] as usize],
                    &mesh.vertices[triangle[2] as usize],
                ];
                let p = v.map(|v| [v.pos.x * pixels_per_point, v.pos.y * pixels_per_point]);
                let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1])
                    - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let min_x = p.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
                let max_x = p.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
                let min_y = p.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                let max_y = p.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
                let left = (min_x.floor().max(0.0) as usize).max(clip_left);
                let right = ((max_x.ceil().max(0.0) as usize) + 1).min(clip_right);
                let top = (min_y.floor().max(0.0) as usize).max(clip_top);
                let bottom = ((max_y.ceil().max(0.0) as usize) + 1).min(clip_bottom);
                let colors = v.map(|v| v.color.to_array().map(|c| c as f32));
                // When every corner has the same texture coordinates the texel is the same
                // across the triangle: the usual case, for anything that is not text or a
                // picture.
                let flat = v[0].uv == v[1].uv && v[1].uv == v[2].uv;
                let flat_texel = texture.sample(v[0].uv.x, v[0].uv.y);

                for y in top..bottom {
                    let py = y as f32 + 0.5;
                    for x in left..right {
                        let px = x as f32 + 0.5;
                        // How close the pixel's centre is to each corner, as shares of 1.
                        let w0 = ((p[1][0] - px) * (p[2][1] - py)
                            - (p[2][0] - px) * (p[1][1] - py))
                            / area;
                        let w1 = ((p[2][0] - px) * (p[0][1] - py)
                            - (p[0][0] - px) * (p[2][1] - py))
                            / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                            continue;
                        }
                        let texel = if flat {
                            flat_texel
                        } else {
                            let u = w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x;
                            let t = w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y;
                            texture.sample(u, t)
                        };
                        let mut source = [0.0f32; 4];
                        for i in 0..4 {
                            let tint = w0 * colors[0][i] + w1 * colors[1][i] + w2 * colors[2][i];
                            source[i] = tint * texel[i] / 255.0;
                        }
                        // Premultiplied colours go over what is there by keeping the share
                        // of it that the new colour does not cover.
                        let keep = 1.0 - source[3] / 255.0;
                        let pixel = &mut frame[y * width + x];
                        for i in 0..3 {
                            pixel[i] = source[i] + pixel[i] * keep;
                        }
                    }
                }
            }
        }
        frame
            .iter()
            .flat_map(|p| {
                [
                    p[0].round().clamp(0.0, 255.0) as u8,
                    p[1].round().clamp(0.0, 255.0) as u8,
                    p[2].round().clamp(0.0, 255.0) as u8,
                    255,
                ]
            })
            .collect()
    }
}
