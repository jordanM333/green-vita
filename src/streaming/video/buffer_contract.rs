//! Checked application-side contract for our single-plane RGB565 AVCDEC adapter.
//! Native retention/alignment guarantees still require Vita validation (see the audit).
use super::{
    DecoderConfig, HW_DECODER_HEIGHT, HW_DECODER_WIDTH, HW_OUTPUT_HEIGHT, HW_OUTPUT_WIDTH,
    VideoTextureTarget,
};
use anyhow::{Context, Result, ensure};

pub(super) const MAX_ACCESS_UNIT_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_CDRAM_BLOCK_BYTES: u32 = 64 * 1024 * 1024;

pub(super) fn dimensions(width: u32, height: u32, max_width: u32, max_height: u32) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && width <= max_width && height <= max_height,
        "invalid video dimensions"
    );
    width
        .checked_mul(height)
        .context("video dimensions overflow")?;
    Ok(())
}

pub(super) fn config(config: DecoderConfig) -> Result<()> {
    dimensions(
        config.decode_width,
        config.decode_height,
        HW_DECODER_WIDTH,
        HW_DECODER_HEIGHT,
    )?;
    dimensions(
        config.output_width,
        config.output_height,
        HW_OUTPUT_WIDTH,
        HW_OUTPUT_HEIGHT,
    )
}

pub(super) fn output(target: VideoTextureTarget, width: u32, height: u32) -> Result<u32> {
    dimensions(width, height, HW_OUTPUT_WIDTH, HW_OUTPUT_HEIGHT)?;
    ensure!(
        target.ptr != 0 && target.ptr.is_multiple_of(2),
        "invalid RGB565 output pointer"
    );
    let row = width.checked_mul(2).context("RGB565 row size overflow")?;
    // framePitch is in pixels, whereas the application records SDL pitch in bytes.
    ensure!(
        target.pitch >= row
            && target.pitch.is_multiple_of(2)
            && target.pitch <= HW_DECODER_WIDTH * 2,
        "invalid RGB565 output stride"
    );
    let bytes = target
        .pitch
        .checked_mul(height)
        .context("RGB565 output size overflow")?;
    ensure!(
        target.capacity >= bytes && target.capacity <= MAX_CDRAM_BLOCK_BYTES,
        "RGB565 output buffer is undersized or exceeds capacity limit"
    );
    target
        .ptr
        .checked_add(bytes as usize)
        .context("RGB565 output address overflow")?;
    Ok(bytes)
}

pub(super) fn allocation(size: u32, alignment: u32) -> Result<u32> {
    ensure!(
        size > 0 && size <= MAX_CDRAM_BLOCK_BYTES && alignment.is_power_of_two(),
        "invalid CDRAM allocation size or alignment"
    );
    size.checked_add(alignment - 1)
        .map(|n| n & !(alignment - 1))
        .filter(|n| *n <= MAX_CDRAM_BLOCK_BYTES)
        .context("CDRAM allocation overflow")
}
