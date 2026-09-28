//! Bounded startup pixel evidence, independent of latency/recovery decisions.
//! No disk IO until teardown. Samples are observations, never playback admission.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

pub(crate) const GRID_W: usize = 32;
pub(crate) const GRID_H: usize = 18;
const N: usize = GRID_W * GRID_H;
const LIMIT: Duration = Duration::from_secs(10);
const INTERVAL: Duration = Duration::from_millis(250);
const ENCODED_CAP: usize = 8 * 1024 * 1024;
const AU_CAP: usize = 1024;
const SAMPLE_CAP: usize = 40;
static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);
static STATE: AtomicU8 = AtomicU8::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Sample {
    at_us: u64,
    pub(crate) generation: u64,
    epoch: u64,
    rtp: u32,
    pub(crate) source: [u16; N],
    pub(crate) uploaded: [u16; N],
    display: [u32; N],
    display_result: i32,
    display_us: u64,
    drawn: bool,
    probe_us: u64,
}
struct Au {
    at_us: u64,
    epoch: u64,
    rtp: u32,
    offset: usize,
    len: usize,
}
struct Capture {
    start: Option<Instant>,
    next_sample: Instant,
    encoded: Vec<u8>,
    aus: Vec<Au>,
    samples: Vec<Sample>,
    encoded_truncated: bool,
    encoded_copy_us: u64,
}
impl Capture {
    fn new(now: Instant) -> Self {
        Self {
            start: None,
            next_sample: now,
            encoded: Vec::with_capacity(ENCODED_CAP),
            aus: Vec::with_capacity(AU_CAP),
            samples: Vec::with_capacity(SAMPLE_CAP),
            encoded_truncated: false,
            encoded_copy_us: 0,
        }
    }
    fn encoded(&mut self, bytes: &[u8], epoch: u64, rtp: u32, now: Instant) {
        let start = *self.start.get_or_insert(now);
        if now.saturating_duration_since(start) >= LIMIT {
            return;
        }
        if self.encoded_truncated
            || self.aus.len() == AU_CAP
            || bytes.len() > ENCODED_CAP - self.encoded.len()
        {
            self.encoded_truncated = true;
            return;
        }
        self.aus.push(Au {
            at_us: now.saturating_duration_since(start).as_micros() as u64,
            epoch,
            rtp,
            offset: self.encoded.len(),
            len: bytes.len(),
        });
        self.encoded.extend_from_slice(bytes);
    }
    fn reserve(&mut self, epoch: u64, rtp: u32, generation: u64, now: Instant) -> Option<Sample> {
        let start = self.start?;
        if now.saturating_duration_since(start) >= LIMIT
            || now < self.next_sample
            || self.samples.len() >= SAMPLE_CAP
        {
            return None;
        }
        self.next_sample = now + INTERVAL;
        Some(Sample {
            at_us: now.saturating_duration_since(start).as_micros() as u64,
            epoch,
            rtp,
            generation,
            source: [0; N],
            uploaded: [0; N],
            display: [0; N],
            display_result: -1,
            display_us: 0,
            drawn: false,
            probe_us: 0,
        })
    }
    fn save_to(&self, destination: &Path, skipped: u64) -> io::Result<()> {
        if destination.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "previous display evidence preserved",
            ));
        }
        let pending = destination.with_extension("partial");
        std::fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| io::Error::other("missing capture parent"))?,
        )?;
        // Never merge or overwrite an earlier incomplete capture.
        std::fs::create_dir(&pending)?;
        std::fs::write(pending.join("submitted.h264"), &self.encoded)?;
        let mut au = io::BufWriter::new(std::fs::File::create(pending.join("access-units.csv"))?);
        writeln!(au, "at_us,decode_epoch,rtp,offset,length")?;
        for a in &self.aus {
            writeln!(
                au,
                "{},{},{},{},{}",
                a.at_us, a.epoch, a.rtp, a.offset, a.len
            )?;
        }
        au.flush()?;
        let mut samples = io::BufWriter::new(std::fs::File::create(pending.join("pixels.csv"))?);
        writeln!(
            samples,
            "at_us,decode_epoch,rtp,generation,drawn,display_result,display_us,probe_us,source_bgr565,uploaded_bgr565,display_abgr8888"
        )?;
        for s in &self.samples {
            write!(
                samples,
                "{},{},{},{},{},{},{},{},",
                s.at_us,
                s.epoch,
                s.rtp,
                s.generation,
                u8::from(s.drawn),
                s.display_result,
                s.display_us,
                s.probe_us
            )?;
            for v in s.source {
                write!(samples, "{v:04x}")?;
            }
            write!(samples, ",")?;
            for v in s.uploaded {
                write!(samples, "{v:04x}")?;
            }
            write!(samples, ",")?;
            for v in s.display {
                write!(samples, "{v:08x}")?;
            }
            writeln!(samples)?;
        }
        samples.flush()?;
        std::fs::write(
            pending.join("README.txt"),
            include_str!("../investigation/DISPLAY-PROBE.txt"),
        )?;
        let mut manifest = std::fs::File::create(pending.join("manifest.json"))?;
        writeln!(
            manifest,
            "{{\"schema\":1,\"source\":\"{}\",\"build\":\"{}\",\"grid\":[{},{}],\"duration_limit_us\":10000000,\"encoded_bytes\":{},\"encoded_truncated\":{},\"lock_skips\":{},\"encoded_copy_us\":{},\"samples\":{},\"pixel_verdict\":\"INCONCLUSIVE_UNTIL_ANALYZED\",\"physical_scanout_verified\":false}}",
            crate::build_info::REVISION,
            crate::build_info::NUMBER,
            GRID_W,
            GRID_H,
            self.encoded.len(),
            self.encoded_truncated,
            skipped,
            self.encoded_copy_us,
            self.samples.len()
        )?;
        manifest.sync_all()?;
        drop(manifest);
        std::fs::rename(pending, destination)
    }
}
fn destination() -> PathBuf {
    PathBuf::from(format!(
        "ux0:data/green-vita-540-test/display-{}",
        crate::build_info::NUMBER
    ))
}
pub(crate) fn begin() {
    let Ok(mut c) = CAPTURE.lock() else {
        return;
    };
    if c.is_some() {
        return;
    }
    if destination().exists() || destination().with_extension("partial").exists() {
        STATE.store(4, Ordering::Relaxed);
        return;
    }
    SKIPPED.store(0, Ordering::Relaxed);
    *c = Some(Capture::new(Instant::now()));
    STATE.store(1, Ordering::Relaxed);
}
pub(crate) fn encoded(bytes: &[u8], epoch: u64, rtp: u32) {
    if STATE.load(Ordering::Relaxed) != 1 {
        return;
    }
    let started = Instant::now();
    if let Ok(mut state) = CAPTURE.try_lock() {
        if let Some(c) = state.as_mut() {
            c.encoded(bytes, epoch, rtp, started);
            c.encoded_copy_us += started.elapsed().as_micros() as u64;
            if c.start
                .is_some_and(|s| started.saturating_duration_since(s) >= LIMIT)
            {
                STATE.store(2, Ordering::Relaxed);
            }
        }
    } else {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn reserve(epoch: u64, rtp: u32, generation: u64) -> Option<Sample> {
    if STATE.load(Ordering::Relaxed) != 1 {
        return None;
    }
    CAPTURE
        .try_lock()
        .ok()?
        .as_mut()?
        .reserve(epoch, rtp, generation, Instant::now())
}
/// The caller supplies a validated, exclusively leased/readable pixel plane.
pub(crate) fn sample_565(
    bytes: &[u8],
    pitch: usize,
    width: usize,
    height: usize,
) -> Option<[u16; N]> {
    if width == 0
        || height == 0
        || width.checked_mul(2)? > pitch
        || pitch.checked_mul(height)? > bytes.len()
    {
        return None;
    }
    let mut out = [0; N];
    for y in 0..GRID_H {
        for x in 0..GRID_W {
            let at = ((2 * y + 1) * height / (2 * GRID_H)) * pitch
                + ((2 * x + 1) * width / (2 * GRID_W)) * 2;
            out[y * GRID_W + x] = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        }
    }
    Some(out)
}
impl Sample {
    pub(crate) fn add_cost(&mut self, us: u64) {
        self.probe_us += us;
    }
}
/// Called after the existing GXM queue-completion wait, before any next draw.
pub(crate) fn finish(mut sample: Sample, drawn: bool) {
    let now = Instant::now();
    sample.drawn = drawn;
    #[cfg(target_os = "vita")]
    {
        // SAFETY: the renderer remains alive and its display callback queue has
        // completed. Query a POD descriptor; validate format/extent before reads.
        unsafe {
            let mut fb: vitasdk_sys::SceDisplayFrameBuf = std::mem::zeroed();
            fb.size = size_of::<vitasdk_sys::SceDisplayFrameBuf>() as u32;
            sample.display_result = vitasdk_sys::sceDisplayGetFrameBuf(
                &mut fb,
                vitasdk_sys::SCE_DISPLAY_SETBUF_NEXTFRAME,
            );
            if sample.display_result >= 0
                && !fb.base.is_null()
                && fb.pixelformat == vitasdk_sys::SCE_DISPLAY_PIXELFORMAT_A8B8G8R8
                && fb.width == 960
                && fb.height == 544
                && (960..=2048).contains(&fb.pitch)
            {
                let pixels = fb.base.cast::<u32>();
                for y in 0..GRID_H {
                    for x in 0..GRID_W {
                        let at = ((2 * y + 1) * 544 / (2 * GRID_H)) * (fb.pitch as usize)
                            + (2 * x + 1) * 960 / (2 * GRID_W);
                        sample.display[y * GRID_W + x] = std::ptr::read_volatile(pixels.add(at));
                    }
                }
            } else if sample.display_result >= 0 {
                sample.display_result = -2;
            }
        }
    }
    sample.probe_us += now.elapsed().as_micros() as u64;
    if let Ok(mut state) = CAPTURE.try_lock() {
        if let Some(c) = state.as_mut() {
            sample.display_us = c
                .start
                .map_or(0, |s| now.saturating_duration_since(s).as_micros() as u64);
            if c.samples.len() < SAMPLE_CAP {
                c.samples.push(sample);
            }
        }
    } else {
        SKIPPED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn status() -> &'static str {
    match STATE.load(Ordering::Relaxed) {
        1 => "Display capture: playing for 10 seconds; then exit normally",
        2 => "Display capture complete — exit stream to save",
        3 => "Display capture saved — copy display folder with VitaShell",
        4 => "Previous display capture preserved — copy display folder",
        5 => "Display capture save failed — partial folder preserved",
        _ => "Display capture ready — start Cloud and play for 15 seconds",
    }
}
pub(crate) fn save() {
    let c = CAPTURE.lock().ok().and_then(|mut c| c.take());
    if let Some(c) = c {
        if c.start.is_none() {
            STATE.store(0, Ordering::Relaxed);
            return;
        }
        let result = c.save_to(&destination(), SKIPPED.load(Ordering::Relaxed));
        STATE.store(if result.is_ok() { 3 } else { 5 }, Ordering::Relaxed);
        if let Err(e) = result {
            eprintln!("Display capture save failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixels_respect_padding_bounds_and_channel_bits() {
        assert!(sample_565(&[0; 10], 8, 4, 2).is_none());
        let mut buf = vec![0xff; 20 * 8];
        for y in 0..8 {
            for x in 0..8 {
                buf[y * 20 + x * 2..y * 20 + x * 2 + 2].copy_from_slice(&0x001fu16.to_le_bytes());
            }
        }
        assert!(
            sample_565(&buf, 20, 8, 8)
                .unwrap()
                .iter()
                .all(|v| *v == 0x001f)
        );
    }
    #[test]
    fn capture_freezes_without_overwrite_and_delayed_export_keeps_identity() {
        let start = Instant::now();
        let mut c = Capture::new(start);
        c.encoded(&[0, 0, 0, 1, 0x65], 7, u32::MAX, start);
        for i in 0..48 {
            let now = start + INTERVAL * i;
            if let Some(mut s) = c.reserve(7, u32::MAX, i.into(), now) {
                s.source.fill(31);
                s.uploaded.fill(31);
                c.samples.push(s);
            }
        }
        assert_eq!(c.samples.len(), 40);
        c.encoded(&[9; 20], 8, 0, start + Duration::from_secs(1000));
        assert_eq!(c.aus.len(), 1);
        let root = std::env::temp_dir().join(format!("green-vita-display-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        c.save_to(&root, 3).unwrap();
        assert!(c.save_to(&root, 0).is_err());
        assert_eq!(
            std::fs::read(root.join("submitted.h264")).unwrap(),
            [0, 0, 0, 1, 0x65]
        );
        assert!(
            std::fs::read_to_string(root.join("access-units.csv"))
                .unwrap()
                .contains("0,7,4294967295,0,5")
        );
        assert!(
            std::fs::read_to_string(root.join("pixels.csv"))
                .unwrap()
                .contains("0,7,4294967295,0,0,-1")
        );
        if let Ok(out) = std::env::var("GREENVITA_DISPLAY_FIXTURE") {
            let out = Path::new(&out);
            let _ = std::fs::remove_dir_all(out);
            std::fs::rename(&root, out).unwrap();
        } else {
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn encoded_capacity_never_keeps_a_partial_access_unit() {
        let now = Instant::now();
        let mut c = Capture::new(now);
        c.encoded(&vec![1; ENCODED_CAP - 2], 0, 0, now);
        c.encoded(&[1; 3], 0, 1, now);
        c.encoded(&[1], 0, 2, now);
        assert!(c.encoded_truncated);
        assert_eq!(c.aus.len(), 1);
        assert_eq!(c.encoded.len(), ENCODED_CAP - 2);
    }
}
