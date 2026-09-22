//! A controlled decoder FIFO, NOT a Vita hardware emulator or latency measurement.
//! Production adapter, worker, metadata and texture ownership run unchanged.
#![allow(dead_code, unused_imports, non_snake_case, unsafe_op_in_unsafe_fn)]
extern crate self as vitasdk_sys;
use std::{collections::{HashMap, VecDeque}, ffi::{c_char,c_void}, sync::{Mutex, LazyLock}};
mod video { include!(concat!(env!("OUT_DIR"), "/video.rs")); }

pub type SceUID = i32;
pub const SCE_AVCDEC_PIXELFORMAT_RGBA565: u32 = 1;
pub const SCE_SYSMODULE_AVCDEC: u32 = 1;
pub const SCE_SYSMODULE_ERROR_INVALID_VALUE: u32 = 0x80020005;
pub const SCE_VIDEODEC_TYPE_HW_AVCDEC: u32 = 1;
pub const SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW: u32 = 1;
pub const SCE_KERNEL_ALLOC_MEMBLOCK_ATTR_HAS_ALIGNMENT: u32 = 1;
pub struct SceVideodecQueryInitInfoHwAvcdec { pub size:u32,pub horizontal:u32,pub vertical:u32,pub numOfRefFrames:u32,pub numOfStreams:u32 }
pub struct SceAvcdecQueryDecoderInfo { pub horizontal:u32,pub vertical:u32,pub numOfRefFrames:u32 }
pub struct SceAvcdecDecoderInfo { pub frameMemSize:u32 }
pub struct SceAvcdecBuf { pub pBuf:*mut c_void,pub size:u32 }
pub struct SceAvcdecCtrl { pub handle:u32,pub frameBuf:SceAvcdecBuf }
#[derive(Clone,Copy)]
pub struct SceVideodecTimeStamp {pub upper:u32,pub lower:u32}
pub struct SceAvcdecAu {pub pts:SceVideodecTimeStamp,pub dts:SceVideodecTimeStamp,pub es:SceAvcdecBuf}
pub struct SceAvcdecFrameOptionRGBA {pub alpha:u32,pub cscCoefficient:u32,pub reserved:[u32;14]}
pub struct SceAvcdecFrameOption {pub rgba:SceAvcdecFrameOptionRGBA}
pub struct SceAvcdecFrame {
    pub pixelType:u32,pub framePitch:u32,pub frameWidth:u32,pub frameHeight:u32,
    pub horizontalSize:u32,pub verticalSize:u32,pub frameCropLeftOffset:u32,
    pub frameCropRightOffset:u32,pub frameCropTopOffset:u32,pub frameCropBottomOffset:u32,
    pub opt:SceAvcdecFrameOption,pub pPicture:[*mut c_void;2],
}
pub struct PictureInfo {pub pts:SceVideodecTimeStamp}
pub struct SceAvcdecPicture {pub size:u32,pub frame:SceAvcdecFrame,pub info:PictureInfo}
pub struct SceAvcdecArrayPicture {pub numOfOutput:u32,pub numOfElm:u32,pub pPicture:*mut *mut SceAvcdecPicture}
pub struct SceKernelFreeMemorySizeInfo {pub size:i32,pub size_user:u32,pub size_cdram:u32,pub size_phycont:u32}
pub struct SceKernelAllocMemBlockOpt {pub size:u32,pub attr:u32,pub alignment:u32,pub uidBaseBlock:i32,pub strBaseBlockName:*const c_char,pub flags:u32,pub reserved:[u32;10]}

#[derive(Default)]
struct Fake {
    next:u32, memory:HashMap<i32,Box<[u8]>>, decoders:HashMap<u32,VecDeque<u64>>,
    inputs:Vec<u64>, outputs:Vec<u64>, polls:usize, deletes:usize, created:usize,
    // A no-output call retains the input. This is the observed trace behavior.
    suppress_inputs:usize, reject_poll:bool, hold_polls:bool, calls:Vec<bool>,
}
// Gate a real worker hardware call without holding the fake's state mutex.
static CALL_GATE: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>> = Mutex::new(None);
static FAKE: LazyLock<Mutex<Fake>> = LazyLock::new(|| Mutex::new(Fake::default()));
pub unsafe fn sceSysmoduleIsLoaded(_:u32)->i32 {0}
pub unsafe fn sceSysmoduleLoadModule(_:u32)->i32 {0}
pub unsafe fn sceSysmoduleUnloadModule(_:u32)->i32 {0}
pub unsafe fn sceVideodecInitLibrary(_:u32,info:*const SceVideodecQueryInitInfoHwAvcdec)->i32 {
    assert_eq!(((*info).horizontal,(*info).vertical,(*info).numOfRefFrames),(1280,720,1)); 0
}
pub unsafe fn sceVideodecTermLibrary(_:u32)->i32 {0}
pub unsafe fn sceAvcdecQueryDecoderMemSize(_:u32,_:*const SceAvcdecQueryDecoderInfo,out:*mut SceAvcdecDecoderInfo)->i32 {(*out).frameMemSize=4;0}
pub unsafe fn sceAvcdecCreateDecoder(_:u32,c:*mut SceAvcdecCtrl,_:*const SceAvcdecQueryDecoderInfo)->i32 {
    let mut f=FAKE.lock().unwrap();f.next+=1;let id=f.next;(*c).handle=id;
    f.created+=1; f.decoders.insert(id,VecDeque::new());0
}
pub unsafe fn sceAvcdecDeleteDecoder(c:*mut SceAvcdecCtrl)->i32 {
    let mut f=FAKE.lock().unwrap();f.deletes+=1;f.decoders.remove(&(*c).handle);0
}
pub unsafe fn sceAvcdecDecode(c:*const SceAvcdecCtrl,au:*const SceAvcdecAu,ap:*mut SceAvcdecArrayPicture)->i32 {
    let gate = CALL_GATE.lock().unwrap().take();
    if let Some((entered, release)) = gate {
        entered.send(()).unwrap();
        release.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    }
    let mut f=FAKE.lock().unwrap();let au=&*au;let ap=&mut *ap;
    assert_eq!(ap.numOfElm,1);ap.numOfOutput=0;
    let is_poll=au.es.size==0; f.calls.push(is_poll);
    if is_poll {
        assert!(au.es.pBuf.is_null());assert_eq!((au.pts.upper,au.pts.lower),(u32::MAX,u32::MAX));
        f.polls+=1;
        if f.reject_poll {return -1;}
        if f.hold_polls {return 0;}
    } else {
        assert!(!au.es.pBuf.is_null());
        let pts=(u64::from(au.pts.upper)<<32)|u64::from(au.pts.lower);
        f.inputs.push(pts);f.decoders.get_mut(&(*c).handle).unwrap().push_back(pts);
        if f.suppress_inputs>0 {f.suppress_inputs-=1;return 0;}
    }
    if let Some(pts)=f.decoders.get_mut(&(*c).handle).unwrap().pop_front() {
        let picture=&mut **ap.pPicture;
        assert_eq!((picture.frame.frameWidth,picture.frame.frameHeight),(960,544));
        picture.info.pts=SceVideodecTimeStamp{upper:(pts>>32)as u32,lower:pts as u32};
        // Put identity in the actual target so publication tests check pixels too.
        std::ptr::write_unaligned(picture.frame.pPicture[0].cast::<u64>(),pts);
        ap.numOfOutput=1;f.outputs.push(pts);
    }
    0
}
pub unsafe fn sceKernelAllocMemBlock(_:*const c_char,_:u32,size:u32,_:*mut SceKernelAllocMemBlockOpt)->i32 {
    let mut f=FAKE.lock().unwrap();f.next+=1;let id=f.next as i32;
    f.memory.insert(id,vec![0;size as usize].into_boxed_slice());id
}
pub unsafe fn sceKernelGetMemBlockBase(id:i32,p:*mut *mut c_void)->i32 {
    *p=FAKE.lock().unwrap().memory.get_mut(&id).unwrap().as_mut_ptr().cast();0
}
pub unsafe fn sceKernelFreeMemBlock(id:i32)->i32 {FAKE.lock().unwrap().memory.remove(&id);0}
pub unsafe fn sceKernelGetFreeMemorySize(_:*mut SceKernelFreeMemorySizeInfo)->i32 {0}
