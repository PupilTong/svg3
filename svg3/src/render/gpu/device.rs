//! Headless GPU device acquisition.

use super::renderer::RenderError;

/// Acquire a headless wgpu device, or [`RenderError::NoAdapter`] if none.
pub(super) fn acquire_gpu() -> Result<(wgpu::Device, wgpu::Queue), RenderError> {
    let instance = wgpu::Instance::default();
    pollster::block_on(async {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|_| {
                log::debug!("svg3::render: no compatible GPU adapter; render skipped");
                RenderError::NoAdapter
            })?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("svg3 headless device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await?;
        Ok((device, queue))
    })
}
