// Each integration-test binary imports this module separately, so helpers used
// by one binary can look unused in another.
#![allow(dead_code)]

use base64::engine::general_purpose;
use base64::Engine as _;

pub(crate) fn png_data_uri(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header should encode");
        writer
            .write_image_data(rgba)
            .expect("PNG pixels should encode");
    }
    format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(bytes)
    )
}

pub(crate) fn solid_png_data_uri(width: u32, height: u32, color: [u8; 4]) -> String {
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..width * height {
        rgba.extend_from_slice(&color);
    }
    png_data_uri(width, height, &rgba)
}

pub(crate) fn quadrant_png_data_uri(colors: [[u8; 4]; 4]) -> String {
    let mut rgba = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4 {
        for x in 0..4 {
            let index = match (x >= 2, y >= 2) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => 3,
            };
            rgba.extend_from_slice(&colors[index]);
        }
    }
    png_data_uri(4, 4, &rgba)
}
