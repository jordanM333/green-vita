# HA03: verified resource correction, unresolved Cloud blackout

This candidate does not claim that the Cloud blackout or sustained progressive
latency is fixed. It retains the existing live-edge policy and DISPLAY01 capture.
No bitrate, resolution, receive budget, thread priority, recovery deadline or
keyframe pacing has changed. Rollback source is
55b8eb045fe61e88c14e17276fa34a48d4b7b533.

## Confirmed defect and correction

The production surface leaked SDL video textures on every detach/replacement.
The UI painter also leaked textures on replacement and removal. Cargo enables
SDL's `unsafe_textures` feature, whose Texture has no Drop implementation.
Dropping its Rust handle frees neither the SDL texture nor GPU allocation until
the renderer itself dies. This is documented in sdl2 0.38.0 render.rs.

The new test executes production VitaSurface upload, scene drawing, painter and
presentation bookkeeping using SDL software rendering. Before the correction,
SDL_GetNumAllocations increased from 96 to 116 after the second stream had
closed. The CDRAM fake was already empty; that check missed SDL-owned memory.

OwnedTexture now holds its exact TextureCreator, preserving renderer lifetime,
and explicitly destroys the uniquely owned texture on Drop. Video detach still
revokes and waits for decoder leases first. The pinned SDL Vita backend calls
sceGxmFinish before freeing a texture. Partial setup failure and spare-slot
truncation also release their resources. Eighty replacements and repeated
production-surface sessions now have bounded SDL allocations.

Classification: CONFIRMED CONTRIBUTING DEFECT (resource retention). There is no
evidence connecting this leak to the first-second Cloud blackout. It must not be
promoted to that root cause.

## Home photograph

The supplied photograph reports Failed / SigninBlockedByPasswordPrompt /
detail=0 / connect accepted=false. The failure occurs in provisioning before
RTC or decoding. The candidate preserves the service error and presents the
specific console sign-in action instead of generic “Failed to check stream
state.” The regression fixture uses that exact response and verifies there is
one state GET, no connect/media retry and no console/session termination.

This is an error-handling correction, not a bypass of Xbox's sign-in requirement
and not proof that Home gameplay can start while that service rejection persists.

## Additional verification beyond the previous fake frame counters

* Actual production surface: changing pixel colours survive two complete
  sessions beyond one second; expired output becomes black without another
  presentation report; CDRAM and SDL resources are released.
* Actual production H.264 assembly and decoder submission: 180 generated Main
  level 3.2, one-reference, four-slice frames cross RFC 6184 FU-A packetization,
  sequence/timestamp wrap and repeated SPS/PPS/IDRs. Independent FFmpeg software
  decoding produces identical per-frame pixel hashes before and after the
  GreenVita path. This is generated media, not recovered HA02 payload.
* Existing real 5,003-packet timing replay and live-edge regressions remain part
  of the host gate. They do not verify the hardware decoder or GXM pixel output.

## Remaining limit

HA02 records presentation submissions after the reported blackout, but contains
no H.264 payload or correlated decoded/uploaded/framebuffer pixels. The supplied
evidence cannot distinguish black encoded media, native decoder output, DMA
handoff or final GXM rendering. The existing DISPLAY01 probe measures those
boundaries and remains in this candidate. No new capture exercise or Mac
topology is introduced. Neither CI nor the tests above close the original issue.
