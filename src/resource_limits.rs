//! Defaults may be reduced by callers, but never exceed these hard safety ceilings.
use anyhow::{Result, ensure};

#[derive(Clone, Copy)]
pub(crate) struct ResourceLimits {
    pub metadata_bytes: usize,
    pub image_bytes: usize,
    pub image_width: u32,
    pub image_height: u32,
    pub image_pixels: u64,
    pub cache_items: usize,
    pub cache_bytes: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::MAX
    }
}

impl ResourceLimits {
    pub const MAX: Self = Self {
        metadata_bytes: 16 * 1024 * 1024,
        image_bytes: 4 * 1024 * 1024,
        image_width: 4096,
        image_height: 4096,
        image_pixels: 4_000_000,
        cache_items: 2048,
        cache_bytes: 128 * 1024 * 1024,
    };
    pub fn validate(self) -> Result<Self> {
        let max = Self::MAX;
        ensure!(
            self.metadata_bytes > 0
                && self.metadata_bytes <= max.metadata_bytes
                && self.image_bytes > 0
                && self.image_bytes <= max.image_bytes
                && self.image_width > 0
                && self.image_width <= max.image_width
                && self.image_height > 0
                && self.image_height <= max.image_height
                && self.image_pixels > 0
                && self.image_pixels <= max.image_pixels
                && self.cache_items > 0
                && self.cache_items <= max.cache_items
                && self.cache_bytes > 0
                && self.cache_bytes <= max.cache_bytes,
            "invalid resource limits"
        );
        Ok(self)
    }
    pub fn image_dimensions(self, width: u32, height: u32) -> Result<()> {
        self.validate()?;
        ensure!(
            width > 0
                && height > 0
                && width <= self.image_width
                && height <= self.image_height
                && u64::from(width)
                    .checked_mul(u64::from(height))
                    .is_some_and(|n| n <= self.image_pixels),
            "decoded image dimensions exceed limit"
        );
        Ok(())
    }
}
