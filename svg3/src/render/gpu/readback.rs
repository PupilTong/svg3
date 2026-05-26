//! Headless CPU readback: the [`Image`] type the headless renderer
//! produces, plus the GPU → CPU copy routine.

use super::renderer::RenderError;

/// An RGBA8 image produced by a headless render.
#[derive(Debug, Clone)]
pub struct Image {
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
    /// Row-major `RGBA8` pixels: `4 * width * height` bytes, no row padding.
    pub pixels: Vec<u8>,
}

impl Image {
    /// The `RGBA8` bytes of the pixel at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if `(x, y)` is outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "pixel out of bounds");
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

/// Map the readback buffer and copy its rows into a tightly-packed
/// (unpadded) row-major `RGBA8` buffer.
pub(super) fn read_back(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
) -> Result<Vec<u8>, RenderError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::Readback(format!("GPU poll failed: {e:?}")))?;
    receiver
        .recv()
        .map_err(|e| RenderError::Readback(e.to_string()))?
        .map_err(|e| RenderError::Readback(e.to_string()))?;

    let row_bytes = (width * 4) as usize;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    {
        let mapped = buffer.slice(..).get_mapped_range();
        for row in 0..height as usize {
            let start = row * padded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..start + row_bytes]);
        }
    }
    buffer.unmap();
    Ok(pixels)
}
