use anyhow::{Result, bail};
use std::ffi::CString;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use vitasdk_sys::*;

const BLOCK_ALIGNMENT: u32 = 256 * 1024;
const RESERVE_SIZES: [u32; 4] = [
    48 * 1024 * 1024,
    24 * 1024 * 1024,
    16 * 1024 * 1024,
    8 * 1024 * 1024,
];

static RESERVED_CDRAM: AtomicI32 = AtomicI32::new(0);

pub fn reserve_decoder_cdram() {
    if RESERVED_CDRAM.load(Ordering::Relaxed) > 0 {
        return;
    }

    for size in RESERVE_SIZES {
        let name = CString::new("xcloud_avcdec_reserve").expect("static name has no interior NUL");
        // SAFETY: static non-NUL name, fixed aligned/capped size and null optional
        // options; the successful UID is owned solely by RESERVED_CDRAM until released.
        let uid = unsafe {
            sceKernelAllocMemBlock(
                name.as_ptr(),
                SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW,
                size,
                std::ptr::null_mut(),
            )
        };
        if uid >= 0 {
            RESERVED_CDRAM.store(uid, Ordering::Relaxed);
            eprintln!("Reserved {size} bytes of CDRAM for AVCDEC");
            return;
        }
        eprintln!(
            "Failed to reserve {size} bytes of CDRAM for AVCDEC: {uid:#x}; {}",
            free_memory_summary(),
        );
    }
}

pub(crate) fn release_reserved_decoder_cdram() {
    let uid = RESERVED_CDRAM.swap(0, Ordering::Relaxed);
    if uid > 0 {
        // SAFETY: atomic swap transfers the reservation's sole UID ownership here;
        // no decoder/reference buffer was created from this reservation.
        unsafe {
            sceKernelFreeMemBlock(uid);
        }
    }
}

fn free_memory_summary() -> String {
    // SAFETY: initialized SDK POD output, valid size tag, synchronous query;
    // no Rust references or retained pointers cross the call.
    unsafe {
        let mut info = SceKernelFreeMemorySizeInfo {
            size: size_of::<SceKernelFreeMemorySizeInfo>() as i32,
            size_user: 0,
            size_cdram: 0,
            size_phycont: 0,
        };
        let ret = sceKernelGetFreeMemorySize(&mut info);
        if ret < 0 {
            return format!("free_memory=unavailable({ret:#x})");
        }
        format!(
            "free_memory=user:{} cdram:{} phycont:{}",
            info.size_user, info.size_cdram, info.size_phycont
        )
    }
}

pub(crate) struct CdramBlock {
    uid: SceUID,
    pub(crate) ptr: *mut u8,
}

impl CdramBlock {
    pub(crate) fn allocate(name: &str, size: u32) -> Result<Self> {
        let c_name = CString::new(name)?;
        let capacity = super::buffer_contract::allocation(size, BLOCK_ALIGNMENT)?;
        let mut options = SceKernelAllocMemBlockOpt {
            size: size_of::<SceKernelAllocMemBlockOpt>() as u32,
            attr: SCE_KERNEL_ALLOC_MEMBLOCK_ATTR_HAS_ALIGNMENT,
            alignment: BLOCK_ALIGNMENT,
            uidBaseBlock: 0,
            strBaseBlockName: std::ptr::null(),
            flags: 0,
            reserved: [0; 10],
        };
        // SAFETY: NUL-terminated name and initialized options remain live during
        // allocation. The SDK owns the returned aligned, physically contiguous block.
        let uid = unsafe {
            sceKernelAllocMemBlock(
                c_name.as_ptr(),
                SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW,
                capacity,
                &mut options,
            )
        };
        if uid < 0 {
            bail!(
                "sceKernelAllocMemBlock({name:?}) failed for {size} bytes rounded to {capacity} bytes: {uid:#x}; {}",
                free_memory_summary(),
            );
        }

        let mut base: *mut c_void = std::ptr::null_mut();
        // SAFETY: uid is a successful allocation and base is a valid out pointer.
        let ret = unsafe { sceKernelGetMemBlockBase(uid, &mut base) };
        if ret < 0 || base.is_null() {
            // SAFETY: ownership has not escaped; release the successfully allocated block.
            unsafe {
                sceKernelFreeMemBlock(uid);
            }
            bail!("sceKernelGetMemBlockBase({name:?}) failed: {ret:#x}");
        }

        // SAFETY: base addresses a live writable block of the checked capacity.
        // Initialize the complete native allocation before it is handed to a decoder.
        unsafe {
            std::ptr::write_bytes(base.cast::<u8>(), 0, capacity as usize);
        }
        Ok(Self {
            uid,
            ptr: base.cast(),
        })
    }
}

impl Drop for CdramBlock {
    fn drop(&mut self) {
        // SAFETY: this block exclusively owns uid. Surface leases are revoked
        // before its output blocks drop; decoder reference memory drops after decoder.
        unsafe {
            sceKernelFreeMemBlock(self.uid);
        }
    }
}
