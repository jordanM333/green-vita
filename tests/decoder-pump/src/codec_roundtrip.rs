//! Independent software decoder oracle for encoded payload integrity. This is
//! generated H.264, not the unavailable payload of the HA02 hardware capture.
use super::*;
use std::process::Command;

fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        let length = if data[i..].starts_with(&[0, 0, 0, 1]) { 4 }
            else if data[i..].starts_with(&[0, 0, 1]) { 3 } else { i += 1; continue };
        starts.push((i, i + length));
        i += length;
    }
    starts.iter().enumerate().map(|(index, &(_, begin))| {
        let end = starts.get(index + 1).map_or(data.len(), |&(start, _)| start);
        &data[begin..end]
    }).collect()
}

#[test]
fn h264_rtp_worker_output_decodes_to_identical_pixels_after_multiple_idrs() {
    reset();
    let source = Command::new("ffmpeg").args([
        "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=60",
        "-frames:v", "180", "-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency",
        "-profile:v", "main", "-level:v", "3.2", "-x264-params",
        "keyint=60:min-keyint=60:scenecut=0:ref=1:bframes=0:aud=1:repeat-headers=1:slices=4",
        "-f", "h264", "pipe:1",
    ]).output().expect("ffmpeg with libx264 is a required host test prerequisite");
    assert!(source.status.success(), "{}", String::from_utf8_lossy(&source.stderr));
    let mut units: Vec<Vec<&[u8]>> = Vec::new();
    for nal in nal_units(&source.stdout) {
        if nal[0] & 31 == 9 { units.push(Vec::new()); }
        units.last_mut().expect("AUD before first AU").push(nal);
    }
    assert_eq!(units.len(), 180);
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    let mut assembly = crate::video_rtp::VideoRtp::new(1280, 720);
    FAKE.lock().unwrap().capture_encoded = true;
    let mut seq = u16::MAX - 20;
    let origin = Instant::now();
    for (frame, nals) in units.iter().enumerate() {
        // Independent RFC 6184 packetization. Include large CABAC multi-slice
        // pictures, SPS/PPS repeats, FU-A and RTP timestamp/sequence wrap.
        let mut packets = Vec::new();
        for nal in nals {
            if nal.len() <= 1100 { packets.push(nal.to_vec()); continue; }
            let chunks: Vec<_> = nal[1..].chunks(1098).collect();
            for (index, chunk) in chunks.iter().enumerate() {
                let mut payload = vec![(nal[0] & 0xe0) | 28, (nal[0] & 31)
                    | if index == 0 { 0x80 } else { 0 }
                    | if index + 1 == chunks.len() { 0x40 } else { 0 }];
                payload.extend_from_slice(chunk);
                packets.push(payload);
            }
        }
        let now = Instant::now();
        let ts = 0xffff_0000u32.wrapping_add((now.duration_since(origin).as_micros() * 90 / 1000) as u32);
        for (index, payload) in packets.iter().enumerate() {
            worker.observe_media(ts, seq, now, now);
            let packet = crate::rtp::Packet { header: crate::rtp::header::Header {
                timestamp: ts, sequence_number: seq, marker: index + 1 == packets.len(), ..Default::default()
            }, payload: bytes::Bytes::copy_from_slice(payload) };
            let _stats = assembly.receive_at(&worker, packet, now, &mut false);
            seq = seq.wrapping_add(1);
        }
        wait_for(|| output.has_pending_frame());
        let (_, _, _, _, timing) = output.take_latest_for_display().unwrap();
        output.confirm_presentation(timing::PresentedFrame { timing: timing.unwrap(), rendered_at: Instant::now() });
        assert_eq!(FAKE.lock().unwrap().encoded.len(), frame + 1);
    }
    worker.shutdown();
    let encoded: Vec<u8> = FAKE.lock().unwrap().encoded.iter().flatten().copied().collect();
    let temp = std::env::temp_dir().join(format!("greenvita-codec-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let mut checksums = Vec::new();
    for (name, bytes) in [("source.h264", &source.stdout), ("submitted.h264", &encoded)] {
        let path = temp.join(name);
        std::fs::write(&path, bytes).unwrap();
        let decoded = Command::new("ffmpeg").args(["-v", "error", "-err_detect", "explode", "-i"])
            .arg(&path).args(["-f", "framemd5", "pipe:1"]).output().unwrap();
        assert!(decoded.status.success(), "{}", String::from_utf8_lossy(&decoded.stderr));
        assert!(decoded.stderr.is_empty(), "{}", String::from_utf8_lossy(&decoded.stderr));
        let text = String::from_utf8(decoded.stdout).unwrap();
        let frames: Vec<String> = text.lines().filter(|line| !line.starts_with('#'))
            .map(|line| line.rsplit(',').next().unwrap().trim().to_owned()).collect();
        assert_eq!(frames.len(), 180);
        checksums.push(frames);
    }
    assert_eq!(checksums[0], checksums[1], "RTP assembly changed decoded pixels");
    std::fs::remove_dir_all(temp).unwrap();
}
