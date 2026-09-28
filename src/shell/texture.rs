//! Explicit texture ownership for SDL's `unsafe_textures` configuration.
//!
//! In that configuration SDL textures have no Drop implementation: dropping a
//! Rust handle alone retains GPU memory until the entire renderer is destroyed.
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, Texture, TextureCreator, TextureValueError};
use sdl2::video::{Window, WindowContext};
use std::ops::{Deref, DerefMut};

pub(crate) struct OwnedTexture {
    texture: Option<Texture>,
    // The creator retains the exact renderer (and its window) through Rc. It
    // outlives destroy() even if the owning Canvas has already been dropped.
    _creator: TextureCreator<WindowContext>,
}

impl OwnedTexture {
    pub(crate) fn streaming(
        canvas: &Canvas<Window>,
        format: PixelFormatEnum,
        width: u32,
        height: u32,
    ) -> Result<Self, TextureValueError> {
        let creator = canvas.texture_creator();
        let texture = creator.create_texture_streaming(format, width, height)?;
        Ok(Self {
            texture: Some(texture),
            _creator: creator,
        })
    }
}

impl Deref for OwnedTexture {
    type Target = Texture;
    fn deref(&self) -> &Self::Target {
        self.texture.as_ref().expect("live texture owner")
    }
}

impl DerefMut for OwnedTexture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.texture.as_mut().expect("live texture owner")
    }
}

impl Drop for OwnedTexture {
    fn drop(&mut self) {
        if let Some(texture) = self.texture.take() {
            // SAFETY: unique texture owner; _creator keeps its original renderer
            // alive until after this destructor. SDL flushes queued references;
            // the pinned Vita GXM backend finishes GPU work before freeing it.
            unsafe { texture.destroy() };
        }
    }
}

#[cfg(all(test, not(target_os = "vita")))]
mod tests {
    use super::*;

    #[test]
    fn replacements_release_sdl_allocations_and_owner_keeps_renderer_alive() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let mut canvas = video
            .window("texture lifetime", 64, 64)
            .hidden()
            .build()
            .unwrap()
            .into_canvas()
            .software()
            .build()
            .unwrap();
        let mut after_close = None;
        for cycle in 0..80 {
            let mut texture =
                OwnedTexture::streaming(&canvas, PixelFormatEnum::BGR565, 960, 544).unwrap();
            texture
                .with_lock(None, |pixels, _| pixels.fill(cycle))
                .unwrap();
            canvas.copy(&texture, None, None).unwrap();
            canvas.present();
            drop(texture);
            // SAFETY: a no-pointer allocator observation; SDL is initialized.
            let count = unsafe { sdl2::sys::SDL_GetNumAllocations() };
            assert!(count > 0, "SDL allocation tracking unavailable");
            if let Some(previous) = after_close {
                assert!(
                    count <= previous,
                    "texture replacement leaked allocations: {previous} -> {count}"
                );
            }
            after_close = Some(count);
        }
        let texture = OwnedTexture::streaming(&canvas, PixelFormatEnum::BGR565, 64, 64).unwrap();
        drop(canvas);
        // The renderer stays alive through the texture's own creator. This
        // covers VitaSurface's field order and standalone painter destruction.
        drop(texture);
    }
}
