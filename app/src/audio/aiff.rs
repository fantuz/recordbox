//! Frame bounds for the fixed-width AIFF/AIFC formats supported by Symphonia 0.5.
//! Its AIFF reader counts the eight SSND control bytes as audio after consuming them.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Copy)]
pub(crate) struct FrameBounds {
    pub frames: usize,
    /// Complete extra frames an older reader could take from padding/trailing metadata.
    pub legacy_tail: u8,
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Inspect using this same open handle; the ordinary demuxer always starts from byte zero.
pub(crate) fn frame_bounds(file: &mut File) -> io::Result<Option<FrameBounds>> {
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let result = inspect(file);
    file.rewind()?;
    result
}

fn inspect(file: &mut File) -> io::Result<Option<FrameBounds>> {
    let mut form = [0u8; 12];
    match file.read_exact(&mut form) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if &form[..4] != b"FORM" || !matches!(&form[8..12], b"AIFF" | b"AIFC") {
        return Ok(None);
    }
    let aifc = &form[8..12] == b"AIFC";
    let eof = file.metadata()?.len();
    let end = 8 + u64::from(u32::from_be_bytes(form[4..8].try_into().unwrap()));
    if end > eof || end < 12 {
        return Err(invalid("AIFF FORM extends outside the file"));
    }
    let mut pos = 12;
    let mut common = None;
    let mut sound = None;
    // Skip even very large artwork with a seek. Bound header work on malformed input.
    for _ in 0..4096 {
        if pos + 8 > end {
            return Err(invalid("AIFF lacks a complete COMM or SSND chunk"));
        }
        file.seek(SeekFrom::Start(pos))?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)?;
        let size = u64::from(u32::from_be_bytes(chunk[4..8].try_into().unwrap()));
        let data_end = pos + 8 + size;
        pos = data_end + size % 2;
        if pos > end {
            return Err(invalid("AIFF chunk extends outside FORM"));
        }
        match &chunk[..4] {
            b"COMM" => {
                if common.is_some() || size < if aifc { 24 } else { 18 } {
                    return Err(invalid("Invalid AIFF COMM chunk"));
                }
                let mut data = [0u8; 22];
                file.read_exact(&mut data[..if aifc { 22 } else { 18 }])?;
                let channels = u64::from(u16::from_be_bytes(data[..2].try_into().unwrap()));
                let frames = u32::from_be_bytes(data[2..6].try_into().unwrap());
                let bits = u16::from_be_bytes(data[6..8].try_into().unwrap());
                let encoding: &[u8] = if aifc { &data[18..22] } else { b"NONE" };
                let width = match (encoding, bits) {
                    (b"NONE" | b"none", 8 | 16 | 24 | 32)
                    | (b"fl32", 32)
                    | (b"fl64", 64)
                    | (b"sowt" | b"twos" | b"TWOS" | b"SOWT", 16) => u64::from(bits / 8),
                    (b"ALAW" | b"alaw" | b"ULAW" | b"ulaw", _) => 1,
                    // Leave codecs outside the demuxer's fixed-width support to its normal dispatch.
                    _ => return Ok(None),
                };
                if channels == 0 {
                    return Err(invalid("AIFF COMM has no channels"));
                }
                common = Some((frames, channels * width));
            }
            b"SSND" => {
                if sound.is_some() || size < 8 {
                    return Err(invalid("Invalid AIFF SSND chunk"));
                }
                let mut controls = [0; 8];
                file.read_exact(&mut controls)?;
                // The pinned demuxer reports block-aligned storage as unsupported.
                if controls != [0; 8] {
                    return Ok(None);
                }
                sound = Some((size - 8, data_end));
            }
            _ => {}
        }
        if let (Some((frames, stride)), Some((payload, audio_end))) = (common, sound) {
            if u64::from(frames) * stride != payload {
                return Err(invalid("AIFF COMM frame count disagrees with the SSND audio payload"));
            }
            let legacy_tail = ((eof - audio_end).min(8) / stride) as u8;
            return Ok(Some(FrameBounds {
                frames: frames as usize,
                legacy_tail,
            }));
        }
    }
    Err(invalid("AIFF has too many chunks before its audio"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{decode_bounded, decode_file, decode_for_analysis, decode_prefix};

    fn chunk(tag: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
        out.extend(tag);
        out.extend((data.len() as u32).to_be_bytes());
        out.extend(data);
        if data.len() % 2 != 0 {
            out.push(0);
        }
    }

    fn fixture(aifc: bool, frames: usize, declared: usize) -> (Vec<u8>, Vec<f32>) {
        let mut body = if aifc { b"AIFC".to_vec() } else { b"AIFF".to_vec() };
        let mut comm = 1u16.to_be_bytes().to_vec();
        comm.extend((declared as u32).to_be_bytes());
        comm.extend((if aifc { 16u16 } else { 24u16 }).to_be_bytes());
        comm.extend([0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
        if aifc {
            chunk(b"FVER", &[0xa2, 0x80, 0x51, 0x40], &mut body);
            comm.extend(b"sowt\0\0");
        }
        chunk(b"NAME", b"odd", &mut body);
        chunk(b"COMM", &comm, &mut body);
        let mut audio = vec![0; 8];
        let mut expected = vec![];
        for i in 0..frames {
            let value = (i as i32 % 1001) - 256;
            if aifc {
                audio.extend((value as i16).to_le_bytes());
                expected.push(value as f32 / 32768.0);
            } else {
                audio.extend(&value.to_be_bytes()[1..]);
                expected.push(value as f32 / 8388608.0);
            }
        }
        chunk(b"SSND", &audio, &mut body);
        chunk(b"ANNO", b"odd metadata after sound", &mut body);
        let mut form = b"FORM".to_vec();
        form.extend((body.len() as u32).to_be_bytes());
        form.extend(&body);
        (form, expected)
    }

    #[test]
    fn aiff_and_little_endian_aifc_stop_at_last_pcm_frame_with_trailing_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let frames = 9001;
        for (aifc, name, surplus) in [(false, "generated.aiff", 2), (true, "generated.aifc", 4)] {
            let path = dir.path().join(name);
            let (bytes, expected) = fixture(aifc, frames, frames);
            std::fs::write(&path, bytes).unwrap();
            let bounds = frame_bounds(&mut File::open(&path).unwrap()).unwrap().unwrap();
            assert_eq!((bounds.frames, bounds.legacy_tail), (frames, surplus));
            let path = path.to_str().unwrap();
            for decoded in [
                decode_file(path).unwrap(),
                decode_prefix(path, 1).unwrap(),
                decode_bounded(path, 1, || false).unwrap(),
                decode_for_analysis(path, 512 * 1024 * 1024, || false).unwrap(),
            ] {
                assert_eq!(decoded.frames(), frames);
                assert!(!decoded.truncated && decoded.damaged == 0);
                for (pair, &sample) in decoded.samples.chunks_exact(2).zip(&expected) {
                    assert_eq!(pair, [sample, sample]);
                }
            }
        }
    }

    #[test]
    fn contradictory_or_truncated_aiff_never_guesses_a_shorter_track() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.aiff");
        for declared in [100, 150] {
            std::fs::write(&path, fixture(false, 120, declared).0).unwrap();
            assert!(decode_file(path.to_str().unwrap())
                .err()
                .unwrap()
                .to_string()
                .contains("COMM frame count"));
        }
        let mut short = fixture(false, 120, 120).0;
        short.truncate(64);
        std::fs::write(&path, short).unwrap();
        assert!(frame_bounds(&mut File::open(path).unwrap()).is_err());
    }
}
