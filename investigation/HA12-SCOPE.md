# HA12: return the video ceiling to 3 Mbps

HA11-27 (4 Mbps) was worse than HA10. Whenever video arrived above about
3.7 Mbps (around 50, 73, 100 and 116 s), the link stalled within a second:
video and audio both stopped arriving for 8-17 s, ICE round trips rose past
1 s, SCTP retransmitted, and the receive order buffer gave up 4,633 packets
(HA10-26: 0). Each stall ended with a burst of up to 1,733 released packets and
a keyframe recovery. Audio stopping too places the fault in the Wi-Fi link,
not the Xbox encoder: the Vita cannot sustain that rate.

HA12 restores HA10's 3 Mbps ceiling unchanged (bandwidth.rs and its tests);
the sender-backlog model keeps its HA09 3 Mbps case. Build identity HA12.
Busy-scene stutter at 3 Mbps (HA10-26) is therefore expected to return; the
next lever is less video per frame (resolution), which needs approval.

Call-out: bitrate and SDP ceilings change 4 -> 3 Mbps (rollback). Nothing else.
