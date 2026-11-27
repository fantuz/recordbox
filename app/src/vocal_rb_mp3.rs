//! MP3 samples exactly as rekordbox's analysis gets them, for vocal detection: decoded by the
//! installed rekordbox's own libmpg123 (`Contents/MacOS/libmpg123.0.dylib`, the open-source
//! mpg123 library, LGPL, loaded at run time; nothing of it is in this repository).
//!
//! Why: the vocal network is sensitive enough that the last-bit differences between decoders
//! change some levels. With this application's decoder 110 of 132 local MP3s get exactly
//! rekordbox's stored levels; the library rekordbox ships (an older mpg123, generic decoder)
//! decodes slightly differently from both symphonia and current mpg123, and with it the
//! differing tracks match too (see `vocal_rb::bench`).
//!
//! As traced in `rbxfrm::FileReadMp3` (0x102818ed0 … 0x10281a528): flags SEEKBUFFER | FUZZY
//! (gapless off, nothing trimmed), 16-bit signed output, and the one-frame lead rekordbox's
//! timeline has for files with a LAME/Xing frame (`analysis::mp3_timebase`).
//! PREPDECK_VOCAL_MP3=own uses this application's decoder instead.

use std::ffi::{c_char, c_double, c_int, c_long, c_void, CString};

pub const LIB: &str = "/Applications/rekordbox 7/rekordbox.app/Contents/MacOS/libmpg123.0.dylib";

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 4;
const MPG123_OK: c_int = 0;
const MPG123_NEW_FORMAT: c_int = -11;
const MPG123_DONE: c_int = -12;
const MPG123_FLAGS: c_int = 1;
const MPG123_SEEKBUFFER: c_long = 0x100;
const MPG123_FUZZY: c_long = 0x200;
/// No messages on stderr (does not change decoding).
const MPG123_QUIET: c_long = 0x20;
const MPG123_MONO: c_int = 1;
const MPG123_STEREO: c_int = 2;
const MPG123_ENC_SIGNED_16: c_int = 0xD0;

extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

type Handle = *mut c_void;

struct Api {
    new: unsafe extern "C" fn(*const c_char, *mut c_int) -> Handle,
    delete: unsafe extern "C" fn(Handle),
    param: unsafe extern "C" fn(Handle, c_int, c_long, c_double) -> c_int,
    format_none: unsafe extern "C" fn(Handle) -> c_int,
    format: unsafe extern "C" fn(Handle, c_long, c_int, c_int) -> c_int,
    open: unsafe extern "C" fn(Handle, *const c_char) -> c_int,
    getformat: unsafe extern "C" fn(Handle, *mut c_long, *mut c_int, *mut c_int) -> c_int,
    read: unsafe extern "C" fn(Handle, *mut u8, usize, *mut usize) -> c_int,
    close: unsafe extern "C" fn(Handle) -> c_int,
}

// The function pointers are plain C entry points of a thread-safe library (one handle per call).
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn api() -> Option<&'static Api> {
    static A: std::sync::OnceLock<Option<Api>> = std::sync::OnceLock::new();
    A.get_or_init(|| unsafe {
        let path = CString::new(LIB).ok()?;
        let h = dlopen(path.as_ptr(), RTLD_NOW | RTLD_LOCAL);
        if h.is_null() {
            return None;
        }
        let sym = |n: &str| -> Option<*mut c_void> {
            let c = CString::new(n).ok()?;
            let p = dlsym(h, c.as_ptr());
            (!p.is_null()).then_some(p)
        };
        let init: unsafe extern "C" fn() -> c_int = std::mem::transmute(sym("mpg123_init")?);
        if init() != MPG123_OK {
            return None;
        }
        Some(Api {
            new: std::mem::transmute(sym("mpg123_new")?),
            delete: std::mem::transmute(sym("mpg123_delete")?),
            param: std::mem::transmute(sym("mpg123_param")?),
            format_none: std::mem::transmute(sym("mpg123_format_none")?),
            format: std::mem::transmute(sym("mpg123_format")?),
            open: std::mem::transmute(sym("mpg123_open")?),
            getformat: std::mem::transmute(sym("mpg123_getformat")?),
            read: std::mem::transmute(sym("mpg123_read")?),
            close: std::mem::transmute(sym("mpg123_close")?),
        })
    })
    .as_ref()
}

/// Whether MP3s are decoded with rekordbox's library for vocal detection.
pub fn available() -> bool {
    std::env::var("PREPDECK_VOCAL_MP3").map(|v| v != "own").unwrap_or(true) && api().is_some()
}

/// 16-bit interleaved stereo and sample rate of an MP3, on rekordbox's timeline.
/// None when the library is not usable or the file does not decode.
pub fn decode(path: &str) -> Option<(Vec<i16>, u32)> {
    if !available() {
        return None;
    }
    let a = api()?;
    let cpath = CString::new(path).ok()?;
    unsafe {
        let mut err = 0;
        let h = (a.new)(std::ptr::null(), &mut err);
        if h.is_null() {
            return None;
        }
        struct Guard<'a>(&'a Api, Handle);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                unsafe {
                    (self.0.close)(self.1);
                    (self.0.delete)(self.1);
                }
            }
        }
        let g = Guard(a, h);
        (a.param)(h, MPG123_FLAGS, MPG123_SEEKBUFFER | MPG123_FUZZY | MPG123_QUIET, 0.0);
        (a.format_none)(h);
        for rate in [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000] {
            (a.format)(h, rate, MPG123_MONO | MPG123_STEREO, MPG123_ENC_SIGNED_16);
        }
        if (a.open)(h, cpath.as_ptr()) != MPG123_OK {
            return None;
        }
        let (mut rate, mut ch, mut enc): (c_long, c_int, c_int) = (0, 0, 0);
        let mut pcm: Vec<i16> = Vec::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut done = 0usize;
            let r = (a.read)(h, buf.as_mut_ptr(), buf.len(), &mut done);
            if r == MPG123_NEW_FORMAT || rate == 0 {
                (a.getformat)(h, &mut rate, &mut ch, &mut enc);
            }
            if done > 0 {
                if enc != MPG123_ENC_SIGNED_16 || !(1..=2).contains(&ch) {
                    return None;
                }
                let s = buf[..done].chunks_exact(2).map(|c| i16::from_ne_bytes([c[0], c[1]]));
                if ch == 1 {
                    for v in s {
                        pcm.push(v);
                        pcm.push(v);
                    }
                } else {
                    pcm.extend(s);
                }
            }
            if r == MPG123_DONE {
                break;
            }
            if r != MPG123_OK && r != MPG123_NEW_FORMAT {
                // A decode error after audio was read ends the file there (as a damaged tail);
                // one before any audio means the file is not usable.
                if pcm.is_empty() {
                    return None;
                }
                break;
            }
        }
        drop(g);
        if pcm.is_empty() || rate <= 0 {
            return None;
        }
        let lead = crate::analysis::mp3_timebase::lead_frames(path);
        if lead > 0 {
            pcm.splice(0..0, std::iter::repeat(0i16).take(2 * lead));
        }
        // A last frame cut short by the end of the file still takes its length of the timeline
        // in rekordbox (its frame table counts it; mpg123 can't decode it): silence.
        if let Some(spf) = std::fs::read(path).ok().and_then(|d| truncated_last_frame(&d)) {
            pcm.extend(std::iter::repeat(0i16).take(2 * spf));
        }
        Some((pcm, rate as u32))
    }
}

/// Length in bytes and samples of a Layer III frame header.
fn frame_len(h: &[u8]) -> Option<(usize, usize)> {
    const BR1: [usize; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const BR2: [usize; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    if h.len() < 4 || h[0] != 0xFF || h[1] & 0xE0 != 0xE0 || (h[1] >> 1) & 3 != 1 {
        return None;
    }
    let version = (h[1] >> 3) & 3; // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5
    let (bri, sri, pad) = ((h[2] >> 4) as usize, ((h[2] >> 2) & 3) as usize, ((h[2] >> 1) & 1) as usize);
    if version == 1 || bri == 0 || bri == 15 || sri == 3 {
        return None;
    }
    let base = [44100, 48000, 32000][sri];
    let (sr, br, spf) = match version {
        3 => (base, BR1[bri], 1152),
        2 => (base / 2, BR2[bri], 576),
        _ => (base / 4, BR2[bri], 576),
    };
    Some(((spf / 8) * br * 1000 / sr + pad, spf))
}

/// When the chain of frames from the first one runs into the end of the file in the middle of
/// a frame: that frame's samples. Tags at either end are skipped.
fn truncated_last_frame(d: &[u8]) -> Option<usize> {
    let mut p = 0usize;
    if d.len() >= 10 && &d[..3] == b"ID3" {
        p = 10 + ((d[6] as usize & 0x7f) << 21 | (d[7] as usize & 0x7f) << 14 | (d[8] as usize & 0x7f) << 7 | (d[9] as usize & 0x7f));
    }
    while p + 4 <= d.len() && frame_len(&d[p..p + 4]).is_none() {
        p += 1;
    }
    let end = if d.len() >= 128 && &d[d.len() - 128..d.len() - 125] == b"TAG" { d.len() - 128 } else { d.len() };
    while p + 4 <= end {
        let (len, spf) = frame_len(&d[p..p + 4])?;
        if p + len > end {
            return Some(spf);
        }
        p += len;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_cut_off_last_frame_is_found() {
        // 320 kbps, 44.1 kHz, no padding: 1044 bytes per frame.
        let mut f = vec![0u8; 1044];
        f[..4].copy_from_slice(&[0xFF, 0xFB, 0xE0, 0x44]);
        let mut d = b"ID3\x04\0\0\0\0\0\x02xx".to_vec();
        for _ in 0..3 {
            d.extend_from_slice(&f);
        }
        assert_eq!(super::truncated_last_frame(&d), None);
        let mut cut = d.clone();
        cut.extend_from_slice(&f[..600]);
        assert_eq!(super::truncated_last_frame(&cut), Some(1152));
        // An ID3v1 tag after complete frames is not a frame.
        let mut tagged = d.clone();
        tagged.extend_from_slice(b"TAG");
        tagged.extend_from_slice(&[0u8; 125]);
        assert_eq!(super::truncated_last_frame(&tagged), None);
    }

    #[test]
    fn missing_file_is_refused() {
        // Whether or not rekordbox is installed: no samples for a file that is not there.
        assert!(super::decode("/nonexistent/recordbox-test.mp3").is_none());
    }
}
