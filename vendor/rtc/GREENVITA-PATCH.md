# RTC source provenance and narrow local extension

Source: crates.io `rtc` **0.20.0-rc.2**, the version in RX Test 31's lockfile.
Archive SHA-256:
`1ad0cdd4dd6b7f1a523b4e862f2dd817d5e58bcd3f6ea0c8ffb70b81a3505774`.
The downloaded archive was checked against that lockfile before extraction.
LICENSE-MIT and LICENSE-APACHE are copied from the corresponding upstream tag
(webrtc-rs/rtc v0.20.0-rc.2); the crate archive omitted the license files.

Only `src/data_channel/mod.rs` has a source change: expose queued plus SCTP
unacknowledged payload bytes, and a binary-send admission limit using that count.
Existing send methods and ICE, DTLS, SRTP, RTP and SCTP implementations are unchanged.
Root Cargo.lock only switches `rtc` to this path; all dependency versions and the
existing Vita `ring` / `rtc-shared` patches remain unchanged.

The API deliberately says **outstanding payload**, not W3C `bufferedAmount`:
this SCTP implementation counts payload until it is acknowledged. The count
also includes this channel's endpoint writes waiting for `poll_write`, including
the one-byte wire representation of an empty message. `poll_write` synchronously
drains the intermediate handlers, so public callers cannot observe a partially
moved application write between those handlers. Stream/association lookup errors
fail admission; they must never be interpreted as an empty queue.

Threshold events are insufficient for admission: SCTP's events are drained by
the RTC read handler, so a high-buffer notification can itself wait for incoming
traffic. `send()` success previously meant local acceptance, not prompt delivery.

Verification: `cd tests/rtc-transport && cargo test --locked --target x86_64-unknown-linux-gnu -- --nocapture`.
The full peer test exercises ICE/DTLS/SCTP, pre-poll writes, in-flight bytes,
acknowledgement release, feedback/controller capacity, rejected-message absence,
and closed channels. Separate virtual-clock tests use the actual SCTP dependency
to reproduce the unbounded backlog and repeat two-second outages.

This is a byte bound, not a universal delivery deadline. It cannot eliminate
network transit time or delay imposed by other streams sharing the association.
It does not establish how Xbox reacts to delayed presentation feedback.
