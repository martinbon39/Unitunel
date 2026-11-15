use image::{imageops, Rgba, RgbaImage};
use std::path::Path;

/// One monitor, in the physical pixel space of the window manager.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

impl Screen {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }
}

/// Grabs the monitor; returns the image and the factor from physical pixels to image pixels.
pub fn grab(s: &Screen) -> Option<(RgbaImage, f32)> {
    let (cx, cy) = (s.x + s.w as i32 / 2, s.y + s.h as i32 / 2);
    // xcap addresses monitors in points on macOS and in physical pixels elsewhere.
    let (qx, qy) = if cfg!(target_os = "macos") { ((cx as f64 / s.scale) as i32, (cy as f64 / s.scale) as i32) } else { (cx, cy) };
    let m = xcap::Monitor::from_point(qx, qy).ok()?;
    let img = m.capture_image().ok()?;
    let k = img.width() as f32 / s.w.max(1) as f32;
    Some((img, k))
}

const INK: Rgba<u8> = Rgba([10, 10, 10, 255]);
const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

fn put(img: &mut RgbaImage, x: i32, y: i32, c: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

fn disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, c: Rgba<u8>) {
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if (dx * dx + dy * dy) as f32 <= r * r {
                put(img, cx as i32 + dx, cy as i32 + dy, c);
            }
        }
    }
}

fn ring(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, w: f32, c: Rgba<u8>) {
    let ro = (r + w).ceil() as i32;
    for dy in -ro..=ro {
        for dx in -ro..=ro {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d >= r && d <= r + w {
                put(img, cx as i32 + dx, cy as i32 + dy, c);
            }
        }
    }
}

/// Click marker: white ring with ink outline and a center dot, readable on any background.
pub fn mark_click(img: &mut RgbaImage, x: f32, y: f32, scale: f32) {
    let r = 20.0 * scale;
    let w = 3.0 * scale;
    ring(img, x, y, r - 1.5 * scale, w + 3.0 * scale, INK);
    ring(img, x, y, r, w, WHITE);
    disc(img, x, y, 5.0 * scale, INK);
    disc(img, x, y, 3.0 * scale, WHITE);
}

/// Strokes as polylines in image pixels: thick ink line under a white line.
pub fn draw_strokes(img: &mut RgbaImage, strokes: &[Vec<(f32, f32)>], scale: f32) {
    for (r, c) in [(5.0 * scale, INK), (3.0 * scale, WHITE)] {
        for s in strokes {
            for pair in s.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                let steps = (len / (r * 0.5)).ceil().max(1.0) as usize;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32;
                    disc(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, r, c);
                }
            }
            if s.len() == 1 {
                disc(img, s[0].0, s[0].1, r, c);
            }
        }
    }
}

pub fn save_jpeg(img: &RgbaImage, path: &Path, max_w: u32) -> std::io::Result<()> {
    let img = if img.width() > max_w {
        let h = (img.height() as f64 * max_w as f64 / img.width() as f64) as u32;
        imageops::resize(img, max_w, h, imageops::FilterType::Triangle)
    } else {
        img.clone()
    };
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let f = std::fs::File::create(path)?;
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), 85);
    enc.encode_image(&rgb).map_err(std::io::Error::other)
}

pub fn crop_around(img: &RgbaImage, x: f32, y: f32, w: u32, h: u32) -> RgbaImage {
    let w = w.min(img.width());
    let h = h.min(img.height());
    let x0 = (x as i64 - w as i64 / 2).clamp(0, (img.width() - w) as i64) as u32;
    let y0 = (y as i64 - h as i64 / 2).clamp(0, (img.height() - h) as i64) as u32;
    imageops::crop_imm(img, x0, y0, w, h).to_image()
}
