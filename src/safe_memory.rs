use anyhow::{Result, bail};
use std::ffi::c_void;
use vitasdk_sys::{
    SceAppUtilBootParam, SceAppUtilInitParam, sceAppUtilInit, sceAppUtilLoadSafeMemory,
    sceAppUtilSaveSafeMemory, sceAppUtilShutdown,
};

/// Keeps AppUtil initialized for every Safe Memory access made by the application.
pub struct AppUtil;

impl AppUtil {
    pub fn initialize() -> Result<Self> {
        // SAFETY: the SDK specifies zero-initialized input POD structs; neither contains Rust references.
        let mut init: SceAppUtilInitParam = unsafe { std::mem::zeroed() };
        // SAFETY: same POD initialization requirement as init above.
        let mut boot: SceAppUtilBootParam = unsafe { std::mem::zeroed() };
        // SAFETY: both structs are initialized and live through this synchronous call.
        let result = unsafe { sceAppUtilInit(&mut init, &mut boot) };
        if result < 0 {
            bail!("sceAppUtilInit failed: {result:#x}");
        }
        Ok(Self)
    }
}

impl Drop for AppUtil {
    fn drop(&mut self) {
        // SAFETY: the single AppUtil owner outlives all workers and Safe Memory calls.
        unsafe {
            sceAppUtilShutdown();
        }
    }
}

pub fn load<const N: usize>(offset: i64) -> Result<[u8; N]> {
    validate_range(offset, N)?;
    let mut data = [0u8; N];
    // SAFETY: validated app-owned range, initialized writable N-byte array; SDK does not retain it.
    let result =
        unsafe { sceAppUtilLoadSafeMemory(data.as_mut_ptr().cast::<c_void>(), N as u32, offset) };
    if result < 0 {
        bail!("sceAppUtilLoadSafeMemory failed: {result:#x}");
    }
    Ok(data)
}

pub fn save(offset: i64, data: &[u8]) -> Result<()> {
    validate_range(offset, data.len())?;
    // SAFETY: validated range and live input slice; SDK signature is mutable but save reads it synchronously.
    let result = unsafe {
        sceAppUtilSaveSafeMemory(
            data.as_ptr().cast_mut().cast::<c_void>(),
            data.len() as u32,
            offset,
        )
    };
    if result < 0 {
        bail!("sceAppUtilSaveSafeMemory failed: {result:#x}");
    }
    Ok(())
}

/// Only the application-owned key record is accessible through this adapter.
fn validate_range(offset: i64, length: usize) -> Result<()> {
    let offset =
        usize::try_from(offset).map_err(|_| anyhow::anyhow!("invalid Safe Memory offset"))?;
    anyhow::ensure!(
        length > 0 && offset.checked_add(length).is_some_and(|end| end <= 40),
        "Safe Memory access exceeds application key record"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_invalid_safe_memory_ranges_before_native_call() {
        for (offset, length) in [(-1, 1), (0, 0), (0, usize::MAX), (39, 2), (i64::MAX, 1)] {
            assert!(super::validate_range(offset, length).is_err());
        }
        assert!(super::validate_range(0, 40).is_ok());
    }
}
