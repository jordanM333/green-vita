use super::*;
use std::{sync::Arc, time::{Duration,Instant}};
use crate::FAKE;
#[path = "burst_replay.rs"]
mod burst_replay;
#[path = "codec_roundtrip.rs"]
mod codec_roundtrip;
fn config()->DecoderConfig { DecoderConfig{decode_width:1280,decode_height:720,output_width:960,output_height:544} }
fn reset() { *FAKE.lock().unwrap()=Default::default(); }

#[test]
fn production_surface_upload_draw_and_pixels_survive_startup_expiry_and_restart() {
    // Run the actual upload, texture selection, drawing, egui painter and
    // presentation bookkeeping. SDL software, host memory and synchronous memcpy
    // replace GXM/CDRAM/DMA; this does not emulate Vita firmware or panel scanout.
    reset();
    let sdl = sdl2::init().unwrap();
    let subsystem = sdl.video().unwrap();
    let mut surface = crate::shell::surface::VitaSurface::software_fixture(&subsystem);
    let colors = [0x001fu16, 0x07e0, 0xf800]; // BGR565: red, green, blue.
    let expected = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
    let mut closed_allocations = Vec::new();
    for session_number in 0..2 {
        let output = Arc::new(DirectVideoOutput::new(960, 544));
        output.decoder_ready.store(true, Ordering::Release);
        let session = crate::app::StreamingSession(output.clone());
        surface.sync_video_frame(Some(&session)).unwrap();
        let start = Instant::now();
        for frame in 0..120u32 {
            let now = Instant::now();
            let timestamp = 0xffff_0000u32.wrapping_add((now.duration_since(start).as_micros() * 90 / 1000) as u32);
            output.live_edge.lock().unwrap().observe(timestamp, frame as u16, now, now);
            let lease = output.lock_decode_target().unwrap();
            let target = lease.target;
            let color = colors[(frame as usize + session_number) % colors.len()];
            // SAFETY: the exclusive output lease owns the initialized fake CDRAM
            // allocation for its full advertised capacity until publication.
            let bytes = unsafe { std::slice::from_raw_parts_mut(target.ptr as *mut u8, target.capacity as usize) };
            for pixel in bytes.as_chunks_mut::<2>().0 { pixel.copy_from_slice(&color.to_le_bytes()); }
            let timing = timing::FrameTiming { rtp_timestamp: timestamp, received_at: now,
                submitted_at: now, decoded_at: now, epoch: 0 };
            lease.publish(Some(timing)).unwrap();
            surface.sync_video_frame(Some(&session)).unwrap();
            surface.draw_scene(true).unwrap();
            surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
            let pixels = surface.canvas.read_pixels(sdl2::rect::Rect::new(480, 272, 1, 1), sdl2::pixels::PixelFormatEnum::RGB24).unwrap();
            assert_eq!(pixels, expected[(frame as usize + session_number) % expected.len()], "session {session_number}, frame {frame}");
            let presented = output.presentation.lock().unwrap().take().unwrap();
            assert_eq!(presented.timing.rtp_timestamp, timestamp);
            output.confirm_presentation(presented);
            assert_eq!(output.live_edge_state(), live_edge::State::Live);
            std::thread::sleep(Duration::from_millis(17));
        }
        assert!(start.elapsed() > Duration::from_secs(1));
        std::thread::sleep(policy::MAX_LOCAL_VIDEO_AGE + Duration::from_millis(10));
        assert!(surface.needs_expiry_redraw());
        // HA04: an expired picture is held (last good frame, marked as
        // reconnecting), never blanked mid-session and never re-presented.
        surface.draw_scene(true).unwrap();
        surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
        let center = sdl2::rect::Rect::new(480, 272, 1, 1);
        let last = expected[(119 + session_number) % expected.len()];
        assert_eq!(surface.canvas.read_pixels(center, sdl2::pixels::PixelFormatEnum::RGB24).unwrap(), last);
        assert!(output.video_held(Instant::now()));
        assert!(!surface.needs_expiry_redraw(), "held redraws need no urgent repaint");
        assert!(output.presentation.lock().unwrap().take().is_none());
        // Real session end still blanks and releases everything.
        surface.sync_video_frame(None).unwrap();
        surface.draw_scene(true).unwrap();
        surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
        assert_eq!(surface.canvas.read_pixels(center, sdl2::pixels::PixelFormatEnum::RGB24).unwrap(), [0, 0, 0]);
        assert!(!output.video_held(Instant::now()));
        assert!(FAKE.lock().unwrap().memory.is_empty());
        // SDL's allocator counter observes real texture allocations, unlike
        // the fake CDRAM handle count above.
        closed_allocations.push(unsafe { sdl2::sys::SDL_GetNumAllocations() });
    }
    assert!(closed_allocations.iter().all(|&count| count > 0), "SDL allocation tracking unavailable");
    println!("SDL allocations after production surface detach: {closed_allocations:?}");
    assert!(closed_allocations[1] <= closed_allocations[0], "SDL allocations grew after stream exit: {closed_allocations:?}");
}

#[test]
fn production_surface_holds_the_last_good_frame_through_recovery_until_session_end() {
    // HA03-19: recovery turned the picture black while audio continued. The
    // actual surface now holds the last current picture, marked as
    // reconnecting, without reporting it as presented or as a recovery.
    reset();
    let sdl = sdl2::init().unwrap();
    let subsystem = sdl.video().unwrap();
    let mut surface = crate::shell::surface::VitaSurface::software_fixture(&subsystem);
    let output = Arc::new(DirectVideoOutput::new(960, 544));
    output.decoder_ready.store(true, Ordering::Release);
    let session = crate::app::StreamingSession(output.clone());
    surface.sync_video_frame(Some(&session)).unwrap();
    let center = sdl2::rect::Rect::new(480, 272, 1, 1);
    let pixel = |surface: &crate::shell::surface::VitaSurface| {
        surface.canvas.read_pixels(center, sdl2::pixels::PixelFormatEnum::RGB24).unwrap()
    };
    let start = Instant::now();
    let media_ts = |now: Instant| (now.duration_since(start).as_micros() * 90 / 1000) as u32;
    let mut seq = 0u16;
    // Publish one decoded picture of a solid BGR565 colour and present it.
    let mut show = |surface: &mut crate::shell::surface::VitaSurface, color: u16, epoch: u64| {
        let now = Instant::now();
        let timestamp = media_ts(now);
        output.live_edge.lock().unwrap().observe(timestamp, seq, now, now);
        seq = seq.wrapping_add(1);
        let lease = output.lock_decode_target().unwrap();
        let target = lease.target;
        // SAFETY: the exclusive output lease owns the initialized fake CDRAM
        // allocation for its full advertised capacity until publication.
        let bytes = unsafe { std::slice::from_raw_parts_mut(target.ptr as *mut u8, target.capacity as usize) };
        for value in bytes.as_chunks_mut::<2>().0 { value.copy_from_slice(&color.to_le_bytes()); }
        let timing = timing::FrameTiming { rtp_timestamp: timestamp, received_at: now,
            submitted_at: now, decoded_at: now, epoch };
        lease.publish(Some(timing)).unwrap();
        surface.sync_video_frame(Some(&session)).unwrap();
        surface.draw_scene(true).unwrap();
        surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
        (timestamp, now)
    };
    for _ in 0..30 {
        show(&mut surface, 0x07e0, 0); // green
        let presented = output.presentation.lock().unwrap().take().unwrap();
        output.confirm_presentation(presented);
        std::thread::sleep(Duration::from_millis(16));
    }
    assert_eq!(output.live_edge_state(), live_edge::State::Live);
    assert_eq!(pixel(&surface), [0, 255, 0]);
    // The reference chain breaks: the live edge declares an incident and RTP
    // recovery starts a new decode epoch. Neither may blank the uploaded picture.
    output.live_edge.lock().unwrap().damage();
    assert_eq!(output.live_edge_state(), live_edge::State::AwaitingKeyframe);
    output.invalidate_before_epoch(1);
    assert!(surface.needs_expiry_redraw(), "the switch to the held frame is urgent");
    for _ in 0..20 {
        surface.draw_scene(true).unwrap();
        surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
        assert_eq!(pixel(&surface), [0, 255, 0], "last good frame was not held");
        assert!(output.video_held(Instant::now()), "last good frame not held");
        assert!(!surface.needs_expiry_redraw(), "held redraws must not force GPU work");
        assert!(output.presentation.lock().unwrap().take().is_none(), "held frame re-presented");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(output.live_edge_state(), live_edge::State::AwaitingKeyframe, "false recovery");
    assert_eq!(output.live_edge.lock().unwrap().recovered, 0);
    // A current IDR is admitted; its picture (new epoch) replaces the held
    // frame, is presented, and confirms recovery.
    let now = Instant::now();
    let timestamp = media_ts(now);
    {
        let mut edge = output.live_edge.lock().unwrap();
        edge.observe(timestamp, 9_000, now, now);
        edge.submitted(timestamp, true, now);
        assert_eq!(edge.state(), live_edge::State::AwaitingPicture);
    }
    show(&mut surface, 0x001f, 1); // red
    assert_eq!(pixel(&surface), [255, 0, 0]);
    assert!(!output.video_held(Instant::now()));
    let presented = output.presentation.lock().unwrap().take().expect("current picture presented");
    output.confirm_presentation(presented);
    assert_eq!(output.live_edge_state(), live_edge::State::Live);
    assert_eq!(output.live_edge.lock().unwrap().recovered, 1);
    // Real session end: blank and release every resource.
    surface.sync_video_frame(None).unwrap();
    surface.draw_scene(true).unwrap();
    surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
    assert_eq!(pixel(&surface), [0, 0, 0]);
    assert!(!output.video_held(Instant::now()));
    assert!(FAKE.lock().unwrap().memory.is_empty());
}

#[test]
fn production_surface_draws_catch_up_pictures_live_and_holds_only_after_a_decoder_stall() {
    // HA06-22: after a sender drain, AVCDEC output pictures whose AUs had
    // completed 240-330 ms earlier. Each was held, then expired, so the screen
    // stopped for up to 0.66 s. HA07 draws and presents a picture that is
    // behind but freshly decoded; the hold begins only MAX_LOCAL_VIDEO_AGE
    // after the decoder's last picture.
    reset();
    let sdl = sdl2::init().unwrap();
    let subsystem = sdl.video().unwrap();
    let mut surface = crate::shell::surface::VitaSurface::software_fixture(&subsystem);
    let output = Arc::new(DirectVideoOutput::new(960, 544));
    output.decoder_ready.store(true, Ordering::Release);
    let session = crate::app::StreamingSession(output.clone());
    surface.sync_video_frame(Some(&session)).unwrap();
    let center = sdl2::rect::Rect::new(480, 272, 1, 1);
    let pixel = |surface: &crate::shell::surface::VitaSurface| {
        surface.canvas.read_pixels(center, sdl2::pixels::PixelFormatEnum::RGB24).unwrap()
    };
    let colors = [0x001fu16, 0x07e0]; // BGR565: red, green.
    let expected = [[255, 0, 0], [0, 255, 0]];
    // A drain burst: 40 AUs complete together and the decoder works through
    // them one at a time.
    let burst = Instant::now();
    for frame in 0..40u16 {
        output.live_edge.lock().unwrap().observe(u32::from(frame) * 3000, frame, burst, burst);
    }
    let mut caught_up = 0;
    for frame in 0..40u32 {
        std::thread::sleep(Duration::from_millis(10));
        let now = Instant::now();
        let lease = output.lock_decode_target().unwrap();
        let target = lease.target;
        // SAFETY: the exclusive output lease owns the initialized fake CDRAM
        // allocation for its full advertised capacity until publication.
        let bytes = unsafe { std::slice::from_raw_parts_mut(target.ptr as *mut u8, target.capacity as usize) };
        let color = colors[frame as usize % colors.len()];
        for value in bytes.as_chunks_mut::<2>().0 { value.copy_from_slice(&color.to_le_bytes()); }
        let timing = timing::FrameTiming { rtp_timestamp: frame * 3000, received_at: burst,
            submitted_at: burst, decoded_at: now, epoch: 0 };
        lease.publish(Some(timing)).unwrap();
        surface.sync_video_frame(Some(&session)).unwrap();
        surface.draw_scene(true).unwrap();
        surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
        assert_eq!(pixel(&surface), expected[frame as usize % expected.len()], "frame {frame}");
        assert!(!output.video_held(Instant::now()), "frame {frame} held while catching up");
        let presented = output.presentation.lock().unwrap().take().expect("catch-up picture presented");
        assert_eq!(presented.timing.rtp_timestamp, frame * 3000);
        output.confirm_presentation(presented);
        if now.duration_since(burst) > policy::MAX_LOCAL_VIDEO_AGE {
            caught_up += 1;
        }
    }
    assert!(caught_up >= 10, "only {caught_up} pictures exercised catch-up");
    assert_eq!(output.live_edge_state(), live_edge::State::Live);
    let last = expected[39 % expected.len()];
    // Within the stall limit of the last decode, the picture stays live.
    std::thread::sleep(policy::MAX_LOCAL_VIDEO_AGE / 2);
    assert!(!surface.needs_expiry_redraw());
    assert!(!output.video_held(Instant::now()));
    // Nothing newer from the decoder: hold the last good frame.
    std::thread::sleep(policy::MAX_LOCAL_VIDEO_AGE / 2 + Duration::from_millis(20));
    assert!(surface.needs_expiry_redraw(), "the switch to the held frame is urgent");
    surface.draw_scene(true).unwrap();
    surface.paint_egui(1.0, &[], &egui::TexturesDelta::default()).unwrap();
    assert_eq!(pixel(&surface), last);
    assert!(output.video_held(Instant::now()));
    assert!(output.presentation.lock().unwrap().take().is_none(), "held frame re-presented");
    println!("catch-up burst: 40 pictures drawn live, {caught_up} more than {} ms after their AUs completed", policy::MAX_LOCAL_VIDEO_AGE.as_millis());
    surface.sync_video_frame(None).unwrap();
    assert!(FAKE.lock().unwrap().memory.is_empty());
}

#[test]
fn undersized_output_is_rejected_before_native_decode() {
    reset();
    let (output, _pixels) = surfaces();
    let mut hw = decoder::HwVideoDecoder::new(config()).unwrap();
    let lease = output.lock_decode_target().unwrap();
    let mut target = lease.target;
    target.capacity = 1; // Allocation remains valid, so the old code is safe to test.
    assert!(hw.poll(target).is_err());
    assert!(FAKE.lock().unwrap().calls.is_empty(), "validation happened after native invocation");
}

#[test]
fn invalid_video_contracts_never_reach_native_code() {
    reset();
    for value in [0, 1281, u32::MAX, i32::MIN as u32] {
        let mut cfg = config();
        cfg.decode_width = value;
        assert!(decoder::HwVideoDecoder::new(cfg).is_err());
    }
    assert_eq!(FAKE.lock().unwrap().created, 0);
    let (output, _pixels) = surfaces();
    let mut hw = decoder::HwVideoDecoder::new(config()).unwrap();
    let lease = output.lock_decode_target().unwrap();
    for (ptr, pitch, capacity) in [
        (0, 1920, 960 * 544 * 2), (1, 1920, 960 * 544 * 2),
        (lease.target.ptr, 1919, 960 * 544 * 2),
        (lease.target.ptr, u32::MAX, u32::MAX),
        (usize::MAX - 1, 1920, 960 * 544 * 2),
    ] {
        assert!(hw.poll(VideoTextureTarget { ptr, pitch, capacity }).is_err());
    }
    assert!(FAKE.lock().unwrap().calls.is_empty());
    for size in [0, u32::MAX, 64 * 1024 * 1024 + 1] {
        assert!(memory::CdramBlock::allocate("invalid", size).is_err());
    }
    assert!(memory::CdramBlock::allocate("bad\0name", 1).is_err());
}

#[test]
fn native_error_releases_leases_and_decoder_resources_on_every_cycle() {
    for _ in 0..20 {
        reset();
        let (output, _pixels) = surfaces();
        {
            let mut hw = decoder::HwVideoDecoder::new(config()).unwrap();
            FAKE.lock().unwrap().reject_poll = true;
            let lease = output.lock_decode_target().unwrap();
            assert!(hw.poll(lease.target).err().unwrap().to_string().contains("sceAvcdecDecode"));
        }
        output.clear_targets();
        let fake = FAKE.lock().unwrap();
        assert!(fake.memory.is_empty());
        assert!(fake.decoders.is_empty());
        assert_eq!(fake.created, fake.deletes);
    }
}

#[test]
fn unknown_picture_identity_never_borrows_the_submitted_inputs_epoch() {
    reset();
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    FAKE.lock().unwrap().unknown_pts = true;
    let before = metrics::METRICS.output_pts_unmatched.load(Ordering::Relaxed);
    worker.submit_access_unit(vec![1], Instant::now(), 9000);
    wait_for(|| metrics::METRICS.output_pts_unmatched.load(Ordering::Relaxed) > before);
    worker.shutdown();
    assert!(!output.has_pending_frame(), "unknown old pictures must never be displayed as current");
}
#[test]
fn stale_before_socket_is_rejected_by_real_worker_with_fresh_local_arrival() {
    reset();
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    let now = Instant::now();
    // HA06: video later than LAG_CEILING (2.5 s) is stale; later video plays.
    output.live_edge.lock().unwrap().establish(0, now - Duration::from_secs(3), now - Duration::from_secs(3));
    worker.observe_media(0, 0, now - Duration::from_secs(3), now - Duration::from_secs(3));
    worker.observe_media(1500, 1, now, now);
    let expired = metrics::METRICS.expired_access_unit.load(Ordering::Relaxed);
    worker.submit_access_unit(vec![1], now, 1500);
    wait_for(|| metrics::METRICS.expired_access_unit.load(Ordering::Relaxed) > expired);
    assert!(FAKE.lock().unwrap().inputs.is_empty());
    assert!(!output.has_pending_frame());
    worker.shutdown();
}

#[test]
fn uploaded_texture_expires_and_old_epoch_completion_cannot_close_new_recovery() {
    reset();
    let (output, _pixels) = surfaces();
    let now = Instant::now();
    output.live_edge.lock().unwrap().establish(0, now, now);
    output.live_edge.lock().unwrap().observe(0, 0, now, now);
    let timing = timing::FrameTiming { rtp_timestamp: 0, received_at: now,
        submitted_at: now, decoded_at: now, epoch: 0 };
    assert!(output.can_draw(timing, now));
    assert!(!output.can_draw(timing, now + Duration::from_millis(241)));
    let later = now + Duration::from_secs(3);
    {
        let mut edge = output.live_edge.lock().unwrap();
        edge.observe(1500, 1, later, later);
        edge.observe(180_000, 2, later, later);
        edge.submitted(180_000, true, later);
    }
    output.invalidate_before_epoch(1);
    let fresh = timing::FrameTiming { rtp_timestamp: 180_000, received_at: later, ..timing };
    output.confirm_presentation(timing::PresentedFrame { timing: fresh, rendered_at: later });
    assert_eq!(output.live_edge_state(), live_edge::State::AwaitingPicture);
    assert!(!output.can_draw(fresh, later));
    output.confirm_presentation(timing::PresentedFrame { timing: timing::FrameTiming { epoch: 1, ..fresh }, rendered_at: later });
    assert_eq!(output.live_edge_state(), live_edge::State::Live);
}

fn surfaces()->(Arc<DirectVideoOutput>,Vec<Vec<u8>>) {
    let mut pixels=vec![vec![0;960*544*2];3];
    let output=Arc::new(DirectVideoOutput::new(960,544));
    output.set_targets(pixels.iter_mut().map(|p|VideoTextureTarget {ptr:p.as_mut_ptr()as usize,pitch:1920,capacity:p.len()as u32}).collect());
    (output,pixels)
}

#[test]
fn normal_startup_assembly_decode_and_presentation_survive_setup_and_bad_sr() {
    // Real RTP/parameter parser, queue, decoder adapter and output ownership.
    // Only hardware decode and the final display callback are substituted.
    reset();
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    let mut assembly = crate::video_rtp::VideoRtp::new(1280, 720);
    let setup = Instant::now() - Duration::from_secs(2);
    worker.observe_media(0, 0, setup, setup);
    worker.poll_media(Instant::now());
    assert_eq!(output.live_edge_state(), live_edge::State::Unmeasured);
    let sps: &[u8] = &[0x67,0x42,0xc0,0x20,0xda,0x01,0x40,0x16,0xec,0x04,0x40,0,0,3,0,0x40,0,0,0x1e,0x23,0xc6,0x0c,0xa8];
    let pps: &[u8] = &[0x68,0xce,0x0f,0xc8];
    let idr: &[u8] = &[0x65,0xbb,0xcc];
    let mut seq = 1u16;
    for frame in 0..180u32 {
        let now = Instant::now();
        let ts = 0xffff0000u32.wrapping_add(frame * 1500);
        let nals: Vec<&[u8]> = if frame == 0 { vec![sps,pps,idr] } else { vec![&[0x61,0xaa,0xbb]] };
        for (i,nal) in nals.iter().enumerate() {
            worker.observe_media(ts, seq, now, now);
            let packet = crate::rtp::Packet { header: crate::rtp::header::Header {
                timestamp: ts, sequence_number: seq, marker: i + 1 == nals.len(), ..Default::default()
            }, payload: bytes::Bytes::copy_from_slice(nal) };
            let _stats = assembly.receive_at(&worker, packet, now, &mut false);
            seq = seq.wrapping_add(1);
        }
        wait_for(|| output.has_pending_frame());
        let (_,_,_,_,timing) = output.take_latest_for_display().expect("fresh picture");
        let timing = timing.unwrap();
        let rendered_at = Instant::now();
        assert_eq!(timing.rtp_timestamp, ts);
        assert!(output.can_draw(timing, rendered_at));
        output.confirm_presentation(timing::PresentedFrame { timing, rendered_at });
        let mut edge = output.live_edge.lock().unwrap();
        if frame == 0 { edge.sender_report(ts, 1u64 << 32, now); }
        if frame == 60 { edge.sender_report(ts - 1500, 2u64 << 32, now); }
        assert!(edge.can_present(ts, rendered_at));
        assert_eq!(edge.state(), live_edge::State::Live);
        assert_eq!(edge.incidents, 0);
        drop(edge);
        std::thread::sleep(Duration::from_millis(16));
    }
    worker.shutdown();
    assert_eq!(FAKE.lock().unwrap().inputs.len(), 180);
}
fn wait_for(mut check:impl FnMut()->bool) {
    let end=Instant::now()+Duration::from_secs(2);
    while !check() {assert!(Instant::now()<end,"worker did not make progress");std::thread::sleep(Duration::from_millis(1));}
}

#[test]
fn reproduce_fifo_growth_and_drain_without_another_input_or_reset() {
    reset();let(output,_pixels)=surfaces();let mut hw=decoder::HwVideoDecoder::new(config()).unwrap();
    let now=Instant::now();
    // Legacy one-call-per-input: every withheld output adds one permanent frame.
    for rtp in 1..=70 {
        if rtp<=37 {FAKE.lock().unwrap().suppress_inputs=1;}
        let target=output.lock_decode_target().unwrap();
        let picture=hw.decode(&[1],target.target,rtp,now,now,0).unwrap();
        if rtp>37 {assert_eq!(picture.unwrap().timing.unwrap().rtp_timestamp,rtp-37);}
    }
    {let f=FAKE.lock().unwrap();assert_eq!(f.inputs.len()-f.outputs.len(),37);}
    let input_count=FAKE.lock().unwrap().inputs.len();
    let mut drained=Vec::new();
    loop {
        let target=output.lock_decode_target().unwrap();
        match hw.poll(target.target).unwrap() {
            Some(p)=>drained.push(p.timing.unwrap().rtp_timestamp),None=>break,
        }
    }
    assert_eq!(drained,(34..=70).collect::<Vec<_>>());
    let f=FAKE.lock().unwrap();assert_eq!(f.inputs.len(),input_count);assert_eq!(f.outputs.len(),input_count);assert_eq!(f.deletes,0);
    println!("Controlled FIFO: legacy retains 37 pictures; output-only calls retire all 37 with zero new submissions or resets.");
}

#[test]
fn real_worker_services_a_no_picture_input_and_stops_polling_when_empty() {
    reset();let(output,_pixels)=surfaces();FAKE.lock().unwrap().suppress_inputs=1;
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),1252674455),worker::SubmitResult::Submitted));
    wait_for(||output.has_pending_frame());
    let(_,target,_,_,timing)=output.take_latest_for_display().unwrap();
    assert_eq!(timing.unwrap().rtp_timestamp,1252674455);
    assert_eq!(unsafe{std::ptr::read_unaligned(target.ptr as *const u64)},1252674455);
    wait_for(||FAKE.lock().unwrap().polls==1); // output retires the only pending PTS
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(FAKE.lock().unwrap().polls,1); // no idle spin or empty-queue hardware call
    assert_eq!(FAKE.lock().unwrap().inputs,vec![1252674455]);
    assert_eq!(FAKE.lock().unwrap().deletes,0);
    worker.shutdown();assert_eq!(FAKE.lock().unwrap().deletes,1);
}

#[test]
fn recovery_drains_old_epochs_before_showing_new_picture() {
    reset();let(output,_pixels)=surfaces();
    {let mut f=FAKE.lock().unwrap();f.suppress_inputs=2;f.hold_polls=true;}
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    for rtp in [1252235525,1252240025] {
        assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),rtp),worker::SubmitResult::Submitted));
        wait_for(||FAKE.lock().unwrap().polls>=if rtp==1252235525 {1}else{2});
    }
    assert!(!output.has_pending_frame());
    worker.begin_resync(); FAKE.lock().unwrap().hold_polls=false;
    // New IDR call returns an old picture; independent polls must discard the
    // next old picture and reach the IDR even with no subsequent input at all.
    assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),1252348025),worker::SubmitResult::Submitted));
    wait_for(||output.has_pending_frame());
    let(_,target,_,_,timing)=output.take_latest_for_display().unwrap();let timing=timing.unwrap();
    assert_eq!((timing.rtp_timestamp,timing.epoch),(1252348025,1));
    assert_eq!(unsafe{std::ptr::read_unaligned(target.ptr as *const u64)},1252348025);
    assert_eq!(FAKE.lock().unwrap().inputs,vec![1252235525,1252240025,1252348025]);
    assert_eq!(FAKE.lock().unwrap().deletes,0);worker.shutdown();
}

#[test]
fn rejected_poll_is_visible_once_without_reset_loop_or_duplicate_input() {
    reset();let(output,_pixels)=surfaces();
    {let mut f=FAKE.lock().unwrap();f.reject_poll=true;f.suppress_inputs=1;}
    let before=metrics::METRICS.decoder_poll_failed.load(Ordering::Relaxed);
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    for rtp in 1..=3 {
        assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),rtp),worker::SubmitResult::Submitted));
        wait_for(||FAKE.lock().unwrap().inputs.len()==rtp as usize);
        if rtp==1 { wait_for(||FAKE.lock().unwrap().polls==1); }
        else {wait_for(||output.has_pending_frame());output.take_latest_for_display();}
    }
    assert_eq!(FAKE.lock().unwrap().polls,1);assert_eq!(FAKE.lock().unwrap().created,1);
    assert_eq!(FAKE.lock().unwrap().deletes,0);assert_eq!(metrics::METRICS.decoder_poll_failed.load(Ordering::Relaxed),before+1);
    worker.shutdown();
}

#[test]
fn repeated_deferred_outputs_do_not_accumulate_and_displayed_pixels_stay_owned() {
    reset();let(output,_pixels)=surfaces();
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    let mut displayed:Option<(usize,u64)>=None;
    for rtp in 1..=120 {
        if rtp%3==0 {FAKE.lock().unwrap().suppress_inputs=1;}
        assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),rtp),worker::SubmitResult::Submitted));
        wait_for(||FAKE.lock().unwrap().outputs.len()==rtp as usize);
        wait_for(||output.has_pending_frame());
        if let Some((ptr,pts))=displayed {
            assert_eq!(unsafe{std::ptr::read_unaligned(ptr as *const u64)},pts);
        }
        let(_,target,_,_,timing)=output.take_latest_for_display().unwrap();
        assert_eq!(timing.unwrap().rtp_timestamp,rtp);
        displayed=Some((target.ptr,u64::from(rtp)));
    }
    let f=FAKE.lock().unwrap();assert_eq!(f.inputs,f.outputs);assert_eq!(f.created,1);assert_eq!(f.deletes,0);
    assert_eq!(f.polls,40); // exactly the 40 deferred outputs, no ordinary-case probes
    drop(f);worker.shutdown();
}

fn gate_next_call() -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *crate::CALL_GATE.lock().unwrap() = Some((entered_tx, release_rx));
    (entered_rx, release_tx)
}

#[test]
fn renderer_can_take_completed_pixels_during_a_blocked_hardware_call() {
    reset(); let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    worker.submit_access_unit(vec![1], Instant::now(), 100);
    wait_for(||output.has_pending_frame());
    let (entered, release) = gate_next_call();
    worker.submit_access_unit(vec![1], Instant::now(), 200);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let renderer_output = output.clone();
    let renderer = std::thread::spawn(move || tx.send(renderer_output.take_latest_for_display()).unwrap());
    let picture = rx.recv_timeout(Duration::from_millis(100));
    release.send(()).unwrap(); // release even when asserting the legacy failure
    renderer.join().unwrap();
    let (_, target, _, _, timing) = picture.expect("render blocked behind hardware").unwrap();
    assert_eq!(timing.unwrap().rtp_timestamp, 100);
    wait_for(||FAKE.lock().unwrap().outputs.len()==2);
    assert_eq!(unsafe{std::ptr::read_unaligned(target.ptr as *const u64)},100);
    worker.shutdown();
}

#[test]
fn teardown_waits_for_lease_and_closes_admission_before_freeing_pixels() {
    let (output, _pixels) = surfaces();
    let lease = output.lock_decode_target().unwrap();
    let copy = output.clone(); let (tx, rx) = std::sync::mpsc::channel();
    let teardown = std::thread::spawn(move || { copy.clear_targets(); tx.send(()).unwrap(); });
    wait_for(||output.state.lock().unwrap().targets.is_none());
    assert!(rx.try_recv().is_err());
    assert!(output.lock_decode_target().is_none());
    lease.publish(None);
    rx.recv_timeout(Duration::from_secs(2)).unwrap(); teardown.join().unwrap();
    assert!(!output.has_pending_frame());
    assert!(output.take_latest_for_display().is_none());
}

#[test]
fn two_surface_reuse_withdraws_pending_pixels_even_if_decode_returns_none() {
    let output = DirectVideoOutput::new(960,544);
    output.set_targets(vec![VideoTextureTarget{ptr:0,pitch:1920,capacity:960*544*2};2]);
    output.lock_decode_target().unwrap().publish(None);
    output.take_latest_for_display().unwrap();
    output.lock_decode_target().unwrap().publish(None);
    let lease = output.lock_decode_target().unwrap();
    assert!(output.take_latest_for_display().is_none());
    assert!(!output.has_pending_frame());
    drop(lease);
    assert!(output.take_latest_for_display().is_none());
}

#[test]
fn recovery_keyframe_followed_by_four_frame_burst_does_not_force_second_recovery() {
    reset(); let (output, _pixels) = surfaces();
    FAKE.lock().unwrap().suppress_inputs = 5;
    let (entered, release) = gate_next_call();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    worker.begin_resync();
    worker.submit_access_unit(vec![1], Instant::now(), 4184301226);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    // Exact burst shape from Cloud: four complete AUs while hardware is busy.
    let burst = [4184304286,4184305816,4184307346,4184308786];
    let accepted: Vec<_> = burst.iter().map(|rtp| matches!(
        worker.submit_access_unit(vec![1],Instant::now(),*rtp), SubmitResult::Submitted)).collect();
    release.send(()).unwrap();
    assert!(accepted.iter().all(|accepted|*accepted),"legacy capacity three rejects fourth AU");
    wait_for(||FAKE.lock().unwrap().outputs.len()==5);
    assert!(!worker.take_recovery_request());
    let f=FAKE.lock().unwrap();
    assert_eq!(f.inputs,f.outputs);assert_eq!(f.created,1);assert_eq!(f.deletes,0);
    drop(f); worker.shutdown();
}

#[test]
fn queue_byte_budget_releases_on_dequeue_rejection_and_shutdown() {
    use policy::{QueueReservation, AU_QUEUE_BYTES};
    use std::sync::atomic::AtomicUsize;
    let bytes = Arc::new(AtomicUsize::new(0));
    let one=QueueReservation::acquire(&bytes,AU_QUEUE_BYTES).unwrap();
    assert!(QueueReservation::acquire(&bytes,1).is_none());
    drop(one); assert_eq!(bytes.load(Ordering::Acquire),0);
    reset();let (output,_pixels)=surfaces();let (entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(worker.submit_access_unit(vec![1;AU_QUEUE_BYTES],Instant::now(),2),SubmitResult::Submitted));
    assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),3),SubmitResult::QueueFull));
    release.send(()).unwrap();
    worker.shutdown();
}

#[test]
fn intact_reference_chain_survives_fifty_ms_queue_pressure() {
    reset();let(output,_pixels)=surfaces();let(entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),2);
    std::thread::sleep(Duration::from_millis(60));
    release.send(()).unwrap();
    wait_for(||FAKE.lock().unwrap().outputs.len()==2);
    assert!(!worker.take_recovery_request());
    assert_eq!(FAKE.lock().unwrap().inputs,vec![1,2]);
    assert_eq!(FAKE.lock().unwrap().deletes,0);
    worker.shutdown();
}

#[test]
fn eight_frame_cloud_burst_gets_input_service_before_speculative_poll() {
    reset(); let(output,_pixels)=surfaces();
    FAKE.lock().unwrap().suppress_inputs=1;
    let(entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),3026025493);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    // Cloud delivered these eight consecutive AUs in under 7ms. A hardware
    // call in the supplied trace can take 8-10ms: all eight must be admissible.
    let burst=[3026030083,3026031613,3026033053,3026034583,
        3026036113,3026037553,3026039083,3026040523];
    let accepted: Vec<_>=burst.iter().map(|rtp|matches!(
        worker.submit_access_unit(vec![1],Instant::now(),*rtp),SubmitResult::Submitted)).collect();
    release.send(()).unwrap();
    assert!(accepted.into_iter().all(|a|a),"RX36 six-frame queue rejects an intact burst");
    wait_for(||FAKE.lock().unwrap().outputs.len()==9);
    assert!(!worker.take_recovery_request());
    let f=FAKE.lock().unwrap();
    let (inputs,outputs,first_poll,created,deletes)=(f.inputs.clone(),f.outputs.clone(),
        f.calls.iter().position(|poll|*poll),f.created,f.deletes);
    drop(f);worker.shutdown();
    assert_eq!(inputs,outputs);
    assert_eq!(first_poll,Some(9),
        "all queued inputs can produce output; do not insert speculative polls");
    assert_eq!((created,deletes),(1,0));
}

#[test]
fn sustained_no_picture_input_still_services_output_debt() {
    reset(); let(output,_pixels)=surfaces();
    FAKE.lock().unwrap().suppress_inputs=32;
    let(entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    for rtp in 2..=32 {
        assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),rtp),SubmitResult::Submitted));
    }
    release.send(()).unwrap();
    wait_for(||FAKE.lock().unwrap().outputs.len()==32);
    let f=FAKE.lock().unwrap();
    assert_eq!(f.inputs,f.outputs);
    let mut debt=0;
    for poll in &f.calls {
        if *poll { debt-=1; } else { debt+=1; }
        assert!(debt<=policy::OUTPUT_DEBT_WATERMARK,"input priority must not resurrect growing firmware backlog");
    }
    assert_eq!(debt,0);assert_eq!((f.created,f.deletes),(1,0));
    drop(f);assert!(!worker.take_recovery_request());worker.shutdown();
}

#[test]
fn small_access_units_still_have_a_hard_count_bound() {
    reset();let(output,_pixels)=surfaces();let(entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    for rtp in 2..=(policy::AU_QUEUE_CAPACITY as u32+1) {
        assert!(matches!(worker.submit_access_unit(vec![1],Instant::now(),rtp),SubmitResult::Submitted));
    }
    let rejected=matches!(worker.submit_access_unit(vec![1],Instant::now(),100),SubmitResult::QueueFull);
    release.send(()).unwrap();
    assert!(rejected);worker.shutdown();
}

#[test]
fn replacement_idr_cuts_a_full_queue_without_reset_or_publishing_inflight_old_pixels() {
    reset(); let (output, _pixels) = surfaces(); let (entered, release) = gate_next_call();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    worker.submit_access_unit(vec![1], Instant::now(), 100);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    // Fill both limits while one old decode is blocked. No arbitrary P picture
    // can be removed, but the caller-validated replacement IDR may replace all.
    for rtp in 101..101 + policy::AU_QUEUE_CAPACITY as u32 {
        assert!(matches!(worker.submit_access_unit(
            vec![1; policy::AU_QUEUE_BYTES / policy::AU_QUEUE_CAPACITY],
            Instant::now(), rtp), SubmitResult::Submitted));
    }
    let replacement = worker.submit_refresh_access_unit(
        vec![1; policy::AU_QUEUE_BYTES / 2], Instant::now(), 1000);
    let following = worker.submit_access_unit(
        vec![1; policy::AU_QUEUE_BYTES / 2], Instant::now(), 1001);
    let over_budget = worker.submit_access_unit(vec![1], Instant::now(), 1002);
    let depth = worker.queued_frames();
    release.send(()).unwrap();
    assert!(matches!(replacement, SubmitResult::Submitted));
    assert!(matches!(following, SubmitResult::Submitted));
    assert!(matches!(over_budget, SubmitResult::QueueFull));
    assert_eq!(depth, 2);
    wait_for(|| FAKE.lock().unwrap().outputs.len() == 3);
    wait_for(|| output.state.lock().unwrap().pending.and_then(|entry| entry.3)
        .is_some_and(|timing| timing.rtp_timestamp == 1001));
    let (_, target, generation, _, timing) = output.take_latest_for_display().unwrap();
    assert_eq!((timing.unwrap().rtp_timestamp, timing.unwrap().epoch), (1001, 1));
    assert_eq!(generation, 2, "only the replacement and following picture may be published");
    assert_eq!(unsafe { std::ptr::read_unaligned(target.ptr as *const u64) }, 1001);
    assert_eq!(FAKE.lock().unwrap().inputs, [100, 1000, 1001]);
    assert_eq!(FAKE.lock().unwrap().created, 1);
    assert_eq!(FAKE.lock().unwrap().deletes, 0);
    assert!(!worker.take_recovery_request());
    worker.shutdown();
    assert_eq!(worker.queued_frames(), 0);
    assert!(matches!(worker.submit_access_unit(vec![1], Instant::now(), 2000), SubmitResult::Disconnected));
    assert!(matches!(worker.submit_refresh_access_unit(vec![1], Instant::now(), 2000), SubmitResult::Disconnected));
}

#[test]
fn oversized_replacement_cannot_invalidate_a_playable_reference_chain() {
    reset(); let (output, _pixels) = surfaces(); let (entered, release) = gate_next_call();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    worker.submit_access_unit(vec![1], Instant::now(), 1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.submit_access_unit(vec![1], Instant::now(), 2);
    let rejected = worker.submit_refresh_access_unit(
        vec![1; policy::AU_QUEUE_BYTES + 1], Instant::now(), 1000);
    let depth = worker.queued_frames();
    release.send(()).unwrap();
    assert!(matches!(rejected, SubmitResult::QueueFull));
    assert_eq!(depth, 1);
    wait_for(|| FAKE.lock().unwrap().outputs.len() == 2);
    wait_for(|| output.state.lock().unwrap().pending.and_then(|entry| entry.3)
        .is_some_and(|timing| timing.rtp_timestamp == 2));
    assert_eq!(output.take_latest_for_display().unwrap().4.unwrap().epoch, 0);
    assert_eq!(FAKE.lock().unwrap().inputs, [1,2]);
    worker.shutdown();
}

#[test]
fn old_matched_picture_is_not_published_as_current_media() {
    reset();
    let (output, _pixels) = surfaces();
    let now = Instant::now();
    let old = now - Duration::from_secs(22);
    output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
        rtp_timestamp: 9000, received_at: old, submitted_at: old,
        decoded_at: now, epoch: 0,
    }));
    assert!(!output.has_pending_frame(), "a matched PTS is not proof of freshness");
}

#[test]
fn decoded_output_cannot_replace_a_newer_media_timestamp() {
    reset();
    let (output, _pixels) = surfaces();
    let now = Instant::now();
    for rtp_timestamp in [3000, 1500] {
        output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
            rtp_timestamp, received_at: now, submitted_at: now, decoded_at: now, epoch: 0,
        }));
    }
    assert_eq!(output.take_latest_for_display().unwrap().4.unwrap().rtp_timestamp, 3000);
}

#[test]
fn stale_compressed_work_requests_dependency_safe_recovery_before_hardware() {
    reset();
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    // Beyond the HA07 catch-up limit, not merely behind the stall limit.
    let received = Instant::now() - policy::MAX_LOCAL_CATCH_UP - Duration::from_millis(500);
    worker.submit_access_unit(vec![1], received, 9000);
    wait_for(|| worker.take_recovery_request() || !FAKE.lock().unwrap().inputs.is_empty());
    worker.shutdown();
    assert!(FAKE.lock().unwrap().inputs.is_empty(), "obsolete compressed work entered the decoder");
    assert!(!output.has_pending_frame());
}

#[test]
fn catch_up_decodes_and_shows_work_that_waited_past_the_stall_limit() {
    // HA06-22: while the Xbox drained a backlog, AUs reached AVCDEC 240-330 ms
    // after they completed. The 240 ms rule expired each one, which stopped
    // the screen and forced keyframe waits. HA07 decodes and shows them.
    reset();
    let (output, _pixels) = surfaces();
    let mut worker = VideoDecodeWorker::spawn(config(), output.clone()).unwrap();
    let expired = metrics::METRICS.expired_access_unit.load(Ordering::Relaxed);
    let stale = metrics::METRICS.stale_picture.load(Ordering::Relaxed);
    worker.submit_access_unit(vec![1], Instant::now() - Duration::from_secs(1), 9000);
    wait_for(|| output.has_pending_frame());
    assert!(!worker.take_recovery_request(), "catch-up work is not an incident");
    worker.shutdown();
    assert_eq!(FAKE.lock().unwrap().inputs, [9000]);
    let timing = output.take_latest_for_display().unwrap().4.unwrap();
    let now = Instant::now();
    assert!(now.duration_since(timing.received_at) > policy::MAX_LOCAL_VIDEO_AGE);
    assert!(output.can_draw(timing, now), "behind but freshly decoded is current");
    // With nothing newer from the decoder, the stall limit still applies.
    let stalled = timing.decoded_at + policy::MAX_LOCAL_VIDEO_AGE + Duration::from_millis(1);
    assert!(!output.can_draw(timing, stalled));
    assert!(!output.can_draw(timing, timing.received_at + policy::MAX_LOCAL_CATCH_UP + Duration::from_millis(1)));
    assert_eq!(metrics::METRICS.expired_access_unit.load(Ordering::Relaxed), expired);
    assert_eq!(metrics::METRICS.stale_picture.load(Ordering::Relaxed), stale);
}

#[test]
fn a_picture_past_the_catch_up_limit_is_neither_published_nor_selected() {
    reset();
    let (output, _pixels) = surfaces();
    let now = Instant::now();
    let late = now - policy::MAX_LOCAL_CATCH_UP - Duration::from_millis(1);
    assert!(output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
        rtp_timestamp: 3000, received_at: late, submitted_at: late, decoded_at: now, epoch: 0,
    })).is_none());
    assert!(!output.has_pending_frame());
    // Published within the limit, then taken by the UI after it, while still
    // within the stall limit of its decode.
    let behind = now - policy::MAX_LOCAL_CATCH_UP + Duration::from_millis(100);
    output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
        rtp_timestamp: 6000, received_at: behind, submitted_at: behind, decoded_at: now, epoch: 0,
    })).unwrap();
    assert!(output.take_latest_at(now + Duration::from_millis(200)).is_none());
    assert!(!output.has_pending_frame());
}

#[test]
fn pending_texture_expires_during_ui_stall_and_wrap_advances() {
    reset();
    let (output, _pixels) = surfaces();
    let now = Instant::now();
    for timestamp in [u32::MAX - 100, 1400] {
        output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
            rtp_timestamp: timestamp, received_at: now, submitted_at: now,
            decoded_at: now, epoch: 0,
        })).unwrap();
    }
    assert!(output.take_latest_at(now + Duration::from_secs(1)).is_none());
    assert!(!output.has_pending_frame());
    output.invalidate_before_epoch(1);
    // A new random-access epoch may restart the sender clock.
    output.lock_decode_target().unwrap().publish(Some(timing::FrameTiming {
        rtp_timestamp: 1, received_at: now, submitted_at: now,
        decoded_at: now, epoch: 1,
    })).unwrap();
    output.invalidate_before_epoch(2);
    assert!(!output.has_pending_frame(), "refresh revoked the old pending surface");
}

#[test]
fn twenty_minute_presentation_schedule_stays_bounded_through_overload() {
    // Production surface selection with a virtual UI clock. This is NOT an AVC
    // performance benchmark. Produce 60 and consume 20/60 Hz with periodic stalls.
    // HA07: every 20 s, 100 outputs come from a decoder catching up on AUs
    // that completed 330 ms earlier (HA06-22's drains); they are selected.
    reset();
    let (output, _pixels) = surfaces();
    let origin = Instant::now();
    let mut selected = 0;
    let mut caught_up = 0;
    let mut expired = 0;
    for tick in 0..72_000u64 {
        let now = origin + Duration::from_micros(tick * 16_667);
        let received_at = if tick % 1200 < 100 { now - Duration::from_millis(330) } else { now };
        let timing = timing::FrameTiming { rtp_timestamp: (tick * 1500) as u32,
            received_at, submitted_at: received_at, decoded_at: now, epoch: 0 };
        output.lock_decode_target().unwrap().publish(Some(timing)).unwrap();
        if tick % 3 == 0 {
            let at = now + if tick % 600 == 0 { Duration::from_secs(1) } else { Duration::from_millis(30) };
            match output.take_latest_at(at) {
                Some((_, _, _, _, Some(frame))) => {
                    assert_eq!(frame.rtp_timestamp, timing.rtp_timestamp);
                    assert!(at.duration_since(frame.decoded_at) <= policy::MAX_LOCAL_VIDEO_AGE);
                    assert!(at.duration_since(frame.received_at) <= policy::MAX_LOCAL_CATCH_UP);
                    if at.duration_since(frame.received_at) > policy::MAX_LOCAL_VIDEO_AGE {
                        caught_up += 1;
                    }
                    selected += 1;
                }
                None => expired += 1,
                _ => panic!("missing media identity"),
            }
        }
    }
    assert_eq!(selected, 23_880);
    assert_eq!(caught_up, 1_980);
    assert_eq!(expired, 120);
    println!("20 virtual minutes: 72000 outputs, {selected} current selections ({caught_up} catching up), {expired} expired selections; no historical walk-through");
}
