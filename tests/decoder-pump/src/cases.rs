use super::*;
use std::{sync::Arc, time::{Duration,Instant}};
use crate::FAKE;
fn config()->DecoderConfig { DecoderConfig{decode_width:1280,decode_height:720,output_width:960,output_height:544} }
fn reset() { *FAKE.lock().unwrap()=Default::default(); }
fn surfaces()->(Arc<DirectVideoOutput>,Vec<Vec<u8>>) {
    let mut pixels=vec![vec![0;960*544*2];3];
    let output=Arc::new(DirectVideoOutput::new(960,544));
    output.set_targets(pixels.iter_mut().map(|p|VideoTextureTarget {ptr:p.as_mut_ptr()as usize,pitch:1920,capacity:p.len()as u32}).collect());
    (output,pixels)
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
    let mut inputs=0;
    for poll in &f.calls { if *poll { inputs=0; } else { inputs+=1; assert!(inputs<=2); } }
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
fn admitted_burst_still_expires_after_fifty_ms_without_decoder_reset() {
    reset();let(output,_pixels)=surfaces();let(entered,release)=gate_next_call();
    let mut worker=VideoDecodeWorker::spawn(config(),output.clone()).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),1);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.submit_access_unit(vec![1],Instant::now(),2);
    std::thread::sleep(Duration::from_millis(60));
    release.send(()).unwrap();
    wait_for(||worker.take_recovery_request());
    assert_eq!(FAKE.lock().unwrap().inputs,vec![1]);
    assert_eq!(FAKE.lock().unwrap().deletes,0);
    worker.shutdown();
}
