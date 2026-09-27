//! Safe Memory boundary substitute. Does not assert Vita storage durability.
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
static RECORD: Mutex<[u8; 40]> = Mutex::new([0; 40]);
pub static CALLS: AtomicUsize = AtomicUsize::new(0);
#[repr(C)]
pub struct SceAppUtilInitParam {
    pub reserved: u32,
}
#[repr(C)]
pub struct SceAppUtilBootParam {
    pub reserved: u32,
}
/// # Safety
/// Caller supplies initialized SDK input structures for this synchronous call.
pub unsafe fn initialize(_: *mut SceAppUtilInitParam, _: *mut SceAppUtilBootParam) -> i32 {
    0
}
/// # Safety
/// No Safe Memory access may outlive the AppUtil owner.
pub unsafe fn shutdown() -> i32 {
    0
}
/// # Safety
/// Caller provides `length` writable bytes, matching the SDK signature.
pub unsafe fn load(output: *mut std::ffi::c_void, length: u32, offset: i64) -> i32 {
    CALLS.fetch_add(1, Ordering::SeqCst);
    let record = RECORD.lock().unwrap();
    let start = usize::try_from(offset).unwrap();
    let source = &record[start..start + length as usize];
    // SAFETY: fixture checks the same range and the adapter supplies an initialized slice.
    unsafe {
        std::ptr::copy_nonoverlapping(source.as_ptr(), output.cast(), source.len());
    }
    0
}
/// # Safety
/// Caller provides `length` readable bytes, matching the SDK signature.
pub unsafe fn save(input: *mut std::ffi::c_void, length: u32, offset: i64) -> i32 {
    CALLS.fetch_add(1, Ordering::SeqCst);
    let mut record = RECORD.lock().unwrap();
    let start = usize::try_from(offset).unwrap();
    let target = &mut record[start..start + length as usize];
    // SAFETY: fixture checks the same range and adapter supplies a live input slice.
    unsafe {
        std::ptr::copy_nonoverlapping(input.cast(), target.as_mut_ptr(), target.len());
    }
    0
}
pub use initialize as sceAppUtilInit;
pub use load as sceAppUtilLoadSafeMemory;
pub use save as sceAppUtilSaveSafeMemory;
pub use shutdown as sceAppUtilShutdown;
