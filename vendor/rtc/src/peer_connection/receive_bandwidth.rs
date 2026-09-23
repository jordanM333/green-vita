//! Media-section bandwidth limits sent in every local offer, including chat.
//! REMB is a later estimate; the offer must also declare receiver capacity.


pub(crate) fn limit_video(sdp: &str, maximum_bps: u32) -> String {
    let lines: Vec<_> = sdp.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("m=video ") {
            out.push(lines[i].to_owned()); i += 1; continue;
        }
        let start = i;
        i += 1;
        while i < lines.len() && !lines[i].starts_with("m=") { i += 1; }
        let section = &lines[start..i];
        if section[0].split_whitespace().nth(1).and_then(|s| s.split('/').next()) == Some("0") {
            out.extend(section.iter().map(|s| (*s).to_owned())); continue;
        }
        let mut insert = 1;
        while insert < section.len() && (section[insert].starts_with("i=") || section[insert].starts_with("c=")) { insert += 1; }
        out.extend(section[..insert].iter().map(|s| (*s).to_owned()));
        out.push(format!("b=AS:{}", maximum_bps.div_ceil(1000)));
        out.push(format!("b=TIAS:{maximum_bps}"));
        out.extend(section[insert..].iter().filter(|s|
            !s.starts_with("b=AS:") && !s.starts_with("b=TIAS:"))
            .map(|s| (*s).to_owned()));
    }
    out.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bandwidth_applies_only_to_live_video_and_is_idempotent() {
        let input = "v=0\r\nb=AS:9000\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\nb=AS:128\r\na=sendrecv\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\ni=Game\r\nc=IN IP4 0.0.0.0\r\nb=AS:15000\r\nb=TIAS:15000000\r\nb=RS:100\r\na=recvonly\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sctp-port:5000\r\n";
        let result = limit_video(input, 2_000_000);
        assert!(result.contains("i=Game\r\nc=IN IP4 0.0.0.0\r\nb=AS:2000\r\nb=TIAS:2000000\r\nb=RS:100\r\na=recvonly"));
        assert_eq!(result.split("m=video").next(), input.split("m=video").next());
        assert_eq!(result.split("m=application").nth(1), input.split("m=application").nth(1));
        assert_eq!(result, limit_video(&result, 2_000_000));
        assert_eq!(limit_video("m=video 0/2 UDP/TLS/RTP/SAVPF 102\na=inactive\n", 2_000_000), "m=video 0/2 UDP/TLS/RTP/SAVPF 102\r\na=inactive\r\n");
    }
}
