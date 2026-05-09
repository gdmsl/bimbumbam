//! Glyphon-backed text rendering. We re-export only the surface area `wayland`
//! and `gpu` modules need.

use glyphon::{
    Buffer as TextBuffer, Cache as GlyphonCache, Family, FontSystem, Metrics as TextMetrics,
    Shaping, SwashCache, TextAtlas, TextRenderer, Viewport, Weight,
};

pub struct TextSystem {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    /// Held alive only because [`TextAtlas`] and [`Viewport`] borrow against it.
    _cache: GlyphonCache,
    pub atlas: TextAtlas,
    pub renderer: TextRenderer,
    pub viewport: Viewport,
}

impl TextSystem {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = GlyphonCache::new(device);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let viewport = Viewport::new(device, &cache);
        Self {
            font_system,
            swash_cache,
            _cache: cache,
            atlas,
            renderer,
            viewport,
        }
    }

    pub fn make_buffer(&mut self, text: &str, font_size: f32, width: Option<f32>) -> TextBuffer {
        let mut buffer = TextBuffer::new(
            &mut self.font_system,
            TextMetrics::new(font_size, font_size * 1.2),
        );
        buffer.set_size(&mut self.font_system, width, None);
        let attrs = glyphon::Attrs::new()
            .family(Family::SansSerif)
            .weight(Weight::BOLD);
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }
}
