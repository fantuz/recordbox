//! Where sample 0 of an MP3 sits on rekordbox's timeline.
//!
//! Traced from rekordbox 7.2.19 (`rbxfrm::FileReadMp3`): it builds its own frame table, feeds
//! the frames to mpg123 one by one with gapless decoding off (16-bit output), so the encoder and
//! decoder delay (1105 samples for LAME-style encoders) stays in the audio. The frame scan
//! accepts a frame as the first one only when the two frames after it have the same MPEG
//! version, layer, sample rate and channel mode. A leading Xing/Info/VBRI frame that passes this
//! test is put in the table at position 0 and marked as "before the music", so the audio starts
//! one frame later (1152 samples, 26.1 ms). One that fails it (ffmpeg writes its Info frame as
//! plain stereo in front of joint-stereo audio) is skipped and takes no time at all.
//!
//! Checked on 99 MP3s with rekordbox's own beat detector run on both alignments: the rule
//! predicts the alignment of every file (20 with the extra frame, 79 without).

/// Sample frames of silence rekordbox has in front of the first decoded audio frame.
pub fn lead_frames(path: &str) -> usize {
    use std::io::Read;
    if !path.to_ascii_lowercase().ends_with(".mp3") {
        return 0;
    }
    let Ok(f) = std::fs::File::open(path) else { return 0 };
    let mut d = Vec::new();
    if f.take(4 << 20).read_to_end(&mut d).is_err() {
        return 0;
    }
    lead_frames_in(&d)
}

const BR1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BR2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
const SR1: [u32; 3] = [44100, 48000, 32000];
const SR2: [u32; 3] = [22050, 24000, 16000];

fn valid(h: &[u8]) -> bool {
    h.len() >= 4 && h[0] == 0xFF && h[1] >= 0xF0 && (h[1] >> 1) & 3 != 0 && h[2] >= 0x10 && h[2] >> 4 != 15 && (h[2] >> 2) & 3 != 3
}

/// The header bits the scan compares between neighbouring frames.
fn masked(h: &[u8]) -> [u8; 4] {
    [h[0], h[1] & 0xFE, h[2] & 0x0C, h[3] & 0xC0]
}

/// (frame length in bytes, samples per frame, MPEG-1) for a Layer III header.
fn frame(h: &[u8]) -> Option<(usize, usize, bool)> {
    if !valid(h) || (h[1] >> 1) & 3 != 1 {
        return None;
    }
    let m1 = h[1] & 0x08 != 0;
    let br = if m1 { BR1 } else { BR2 }[(h[2] >> 4) as usize] as f64 * 1000.0;
    let sr = if m1 { SR1 } else { SR2 }[((h[2] >> 2) & 3) as usize] as f64;
    let spf = if m1 { 1152 } else { 576 };
    Some((((spf / 8) as f64 * br / sr) as usize + ((h[2] >> 1) & 1) as usize, spf, m1))
}

pub fn lead_frames_in(d: &[u8]) -> usize {
    let mut p = 0usize;
    while p + 4 <= d.len() {
        let h = &d[p..p + 4];
        if &h[..3] == b"ID3" && p + 10 <= d.len() {
            let sz = ((d[p + 6] as usize & 0x7f) << 21) | ((d[p + 7] as usize & 0x7f) << 14) | ((d[p + 8] as usize & 0x7f) << 7) | (d[p + 9] as usize & 0x7f);
            p += 10 + sz;
            continue;
        }
        let Some((len, spf, m1)) = frame(h) else {
            p += 1;
            continue;
        };
        // The next two frames must look the same.
        let mut q = p + len;
        let mut ok = true;
        for _ in 0..2 {
            match d.get(q..q + 4).and_then(|h2| frame(h2).filter(|_| masked(h2) == masked(h))) {
                Some((l2, _, _)) => q += l2,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            p += 1;
            continue;
        }
        let mono = h[3] >> 6 == 3;
        let side = match (m1, mono) {
            (true, false) => 32,
            (true, true) | (false, false) => 17,
            (false, true) => 9,
        };
        let tag = d.get(p + 4 + side..p + 4 + side + 8).unwrap_or(&[]);
        let info = tag.len() == 8 && (&tag[..4] == b"Info" || (&tag[..4] == b"Xing" && tag[7] & 1 != 0));
        let vbri = d.get(p + 36..p + 40) == Some(b"VBRI");
        return if info || vbri { spf } else { 0 };
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 320 kbps 44.1 kHz frame: header + zeros, optionally with a tag after the side info.
    fn frame_bytes(mode: u8, tag: Option<&[u8; 4]>) -> Vec<u8> {
        let mut f = vec![0u8; 1044];
        f[..4].copy_from_slice(&[0xFF, 0xFB, 0xE0, mode]);
        if let Some(t) = tag {
            f[36..40].copy_from_slice(t);
            f[43] = 1;
        }
        f
    }

    #[test]
    fn info_frame_counts_only_when_it_matches_the_audio_frames() {
        let js = 0x64; // joint stereo, as the audio frames of most files
        // LAME: the Info frame has the audio frames' channel mode -> one frame of lead.
        let mut lame = frame_bytes(js, Some(b"Info"));
        for _ in 0..4 {
            lame.extend(frame_bytes(js, None));
        }
        assert_eq!(lead_frames_in(&lame), 1152);
        // ffmpeg: plain-stereo Info frame in front of joint-stereo audio -> skipped, no lead.
        let mut lavc = frame_bytes(0x00, Some(b"Info"));
        for _ in 0..4 {
            lavc.extend(frame_bytes(js, None));
        }
        assert_eq!(lead_frames_in(&lavc), 0);
        // No tag frame at all.
        let mut plain = vec![];
        for _ in 0..4 {
            plain.extend(frame_bytes(js, None));
        }
        assert_eq!(lead_frames_in(&plain), 0);
        // An ID3v2 tag in front changes nothing; a Xing tag needs its frame-count flag.
        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x0a".to_vec();
        tagged.extend([0u8; 10]);
        tagged.extend(&lame);
        assert_eq!(lead_frames_in(&tagged), 1152);
        let mut xing = frame_bytes(js, Some(b"Xing"));
        xing[43] = 0;
        for _ in 0..4 {
            xing.extend(frame_bytes(js, None));
        }
        assert_eq!(lead_frames_in(&xing), 0);
    }
}
