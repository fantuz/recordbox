//! Stream the requested mix through the EDIT encoders, then verify it before publication.
use super::*;
use crate::audio::Decoded;
use crate::stems::Stems;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

pub(super) struct Work {
    pub path: String,
    pub dir: PathBuf,
    pub part: Part,
    pub setting: u8,
    pub kbps: u32,
    pub audio: Option<Arc<Decoded>>,
    pub stems: Option<Arc<Stems>>,
    pub cache: PathBuf,
    pub model: &'static str,
}

pub(super) struct Staged {
    pub file: tempfile::NamedTempFile,
    pub dir: PathBuf,
    pub plan: Plan,
    pub shift: f64,
    pub source: SourceStamp,
}

/// Keep the exact metadata timestamp as well as the STEMS cache identity. An edit
/// within a single second may leave an older cache key unchanged.
#[derive(PartialEq, Eq)]
pub(super) struct SourceStamp(u64, std::time::SystemTime);
impl SourceStamp {
    pub fn read(path: &str) -> Result<Self, String> {
        let m = std::fs::metadata(path).map_err(|e| format!("Cannot inspect the source: {e}"))?;
        if !m.is_file() {
            return Err("The source is not a regular file.".into());
        }
        Ok(Self(m.len(), m.modified().map_err(|e| e.to_string())?))
    }
}

fn checked_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| c == std::path::Component::ParentDir) {
        return Err("Choose an absolute export folder.".into());
    }
    let roots = [
        crate::rbimport::default_library_dir(),
        dirs::home_dir().map(|h| h.join("Music/rekordbox")),
        dirs::home_dir().map(|h| h.join("Library/Pioneer")),
        Some(crate::db::data_dir().join("stems")),
        Some(crate::db::shared_data_dir_for_inspection().join("stems")),
    ];
    let allowed = |p: &Path| {
        !roots
            .iter()
            .flatten()
            .any(|d| p.starts_with(d) || d.canonicalize().is_ok_and(|d| p.starts_with(d)))
    };
    let existing = path
        .ancestors()
        .find(|d| d.exists())
        .ok_or("Cannot find the export folder's volume.")?;
    let intended = existing
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(path.strip_prefix(existing).map_err(|e| e.to_string())?);
    if !allowed(&intended) {
        return Err("Choose an export folder outside the rekordbox library and separation cache.".into());
    }
    std::fs::create_dir_all(&intended).map_err(|e| format!("Could not create the export folder: {e}"))?;
    let dir = intended.canonicalize().map_err(|e| e.to_string())?;
    if !allowed(&dir) {
        return Err("The export folder resolves inside the rekordbox library or separation cache.".into());
    }
    Ok(dir)
}

enum Vocals {
    Memory(Arc<Stems>),
    Cache {
        file: File,
        len: usize,
        left: Vec<f32>,
        right: Vec<f32>,
        bytes: Vec<u8>,
    },
}

impl Vocals {
    fn open(work: &Work) -> Result<Self, String> {
        if let Some(s) = &work.stems {
            if s.len == 0 || s.len.checked_mul(8) != Some(s.data.len()) {
                return Err("The deck has incomplete stems.".into());
            }
            return Ok(Self::Memory(s.clone()));
        }
        let path = crate::stems::existing_cache_path(&work.cache)
            .ok_or("This track has no completed parts for this model. Load it into a deck and choose STEMS → Separate before exporting.")?;
        let (file, len) = crate::stems::open_cache(&path).map_err(|e| format!("Cannot read the saved stems: {e}"))?;
        Ok(Self::Cache {
            file,
            len,
            left: Vec::new(),
            right: Vec::new(),
            bytes: Vec::new(),
        })
    }

    fn len(&self) -> usize {
        match self {
            Self::Memory(s) => s.len,
            Self::Cache { len, .. } => *len,
        }
    }

    /// Fetch only the two vocal planes needed by this block, including the interpolation lookahead.
    fn window(&mut self, first: usize, end: usize) -> Result<(&[f32], &[f32]), String> {
        match self {
            Self::Memory(s) => Ok((
                &s.data[6 * s.len + first..6 * s.len + end],
                &s.data[7 * s.len + first..7 * s.len + end],
            )),
            Self::Cache {
                file,
                len,
                left,
                right,
                bytes,
            } => {
                bytes.resize((end - first) * 2, 0);
                for (plane, out) in [(6, left as &mut Vec<f32>), (7, right as &mut Vec<f32>)] {
                    let offset = 16 + 2 * (plane * *len as u64 + first as u64);
                    file.seek(SeekFrom::Start(offset))
                        .and_then(|_| file.read_exact(bytes))
                        .map_err(|e| e.to_string())?;
                    out.clear();
                    out.extend(
                        bytes
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0),
                    );
                }
                Ok((left, right))
            }
        }
    }
}

fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("stopped".into())
    } else {
        Ok(())
    }
}

/// Measure the encoder and this installation's MP3 reader together. The probe has no user data.
fn encoder_lead(plan: &Plan, target: &Path, cancel: &AtomicBool) -> Result<usize, String> {
    if !plan.lossy() {
        return Ok(0);
    }
    let n = plan.rate as usize;
    let start = n / 4;
    let mut wave = [0i16; 1024];
    let mut seed = 0x12345678u32;
    for v in &mut wave {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = ((seed >> 17) as i16) - 16384;
    }
    let mut pos = 0usize;
    let file = render::encode_part(target, &plan.enc, plan.ch, n, plan.rate, cancel, |dst| {
        for frame in dst.chunks_exact_mut(usize::from(plan.ch)) {
            let value = pos.checked_sub(start).and_then(|i| wave.get(i)).copied().unwrap_or(0);
            frame.fill(i32::from(value));
            pos += 1;
        }
        Ok(())
    })?;
    let back = crate::audio::decode_bounded(&file.path().to_string_lossy(), 3, || cancel.load(Ordering::Relaxed))
        .map_err(|e| e.to_string())?;
    let win_energy: f64 = wave.iter().map(|&x| f64::from(x).powi(2)).sum();
    let mut best = (0, 0.0);
    for lag in 0..=6000 {
        let Some(window) = back.samples.get((start + lag) * 2..(start + lag + wave.len()) * 2) else {
            break;
        };
        if lag % 256 == 0 {
            check(cancel)?;
        }
        let (mut dot, mut energy) = (0.0, 0.0);
        for (frame, &x) in window.chunks_exact(2).zip(&wave) {
            let val = f64::from(frame[0]);
            dot += f64::from(x) * val;
            energy += val * val;
        }
        let score = dot / (energy * win_energy).sqrt().max(1e-12);
        if score > best.1 {
            best = (lag, score);
        }
    }
    if best.1 < 0.45 {
        return Err(
            "Could not measure the encoder delay. Choose FLAC, WAV or AIFF to preserve the cue timeline.".into(),
        );
    }
    Ok(best.0)
}

pub(super) fn stage(w: Work, cancel: &AtomicBool) -> Result<Staged, String> {
    check(cancel)?;
    let source = SourceStamp::read(&w.path)?;
    let dir = checked_dir(&w.dir)?;
    // Check the cache before decoding a possibly large off-deck source.
    let mut vocals = Vocals::open(&w)?;
    let audio = match w.audio {
        Some(a) => a,
        None => Arc::new(
            crate::audio::decode_bounded(&w.path, 10800, || cancel.load(Ordering::Relaxed))
                .map_err(|e| format!("Cannot read the complete track within the PCM limit: {e}"))?,
        ),
    };
    if audio.sr == 0 || audio.frames() == 0 || audio.truncated || audio.damaged != 0 {
        return Err("The complete original audio is required for a timeline-aligned export.".into());
    }
    let src = probe_source(&w.path, w.kbps).ok_or("Cannot determine the original audio format.")?;
    let plan = plan(w.setting, &SourceInfo { rate: audio.sr, ..src });
    let target = dir.join(format!("stem.{}", plan.ext));
    let src_len = audio.frames();
    let len = vocals.len();
    let stem_rate = crate::stems::htdemucs::SAMPLE_RATE;
    let expected = (src_len as f64 * f64::from(stem_rate) / f64::from(audio.sr)).round() as usize;
    if len == 0 || len.abs_diff(expected) > 1 {
        return Err(
            "The saved parts and the original have different durations. Separate this version of the track again."
                .into(),
        );
    }
    let frames = (src_len as f64 * f64::from(plan.rate) / f64::from(audio.sr)).round() as usize;
    if frames == 0 {
        return Err("The track is too short to export.".into());
    }
    let lead = encoder_lead(&plan, &target, cancel)?;
    // Preserve even a quiet or silent lead-in: it can contain an independently audible
    // vocal part, and the deck's cues refer to the original timeline.
    let mut at = 0usize;
    let scale = (1u32 << (plan.enc.bits - 1)) as f32;
    let sample = |x: f32| (x * scale).round().clamp(-scale, scale - 1.0) as i32;
    let stem_ratio = f64::from(stem_rate) / f64::from(plan.rate);
    let src_ratio = f64::from(audio.sr) / f64::from(plan.rate);
    // At low source rates a whole output block can span many stem frames. Bound each read.
    let max_chunk = (4096.0 / stem_ratio.max(1.0)).max(1.0) as usize * usize::from(plan.ch);
    let file = render::encode_part(&target, &plan.enc, plan.ch, frames, plan.rate, cancel, |out| {
        for block in out.chunks_mut(max_chunk) {
            check(cancel)?;
            let n = block.len() / usize::from(plan.ch);
            let first = ((at as f64 * stem_ratio) as usize).min(len - 1);
            let end = (((at + n - 1) as f64 * stem_ratio) as usize).saturating_add(2).min(len);
            let (left, right) = vocals.window(first, end)?;
            for output in block.chunks_exact_mut(usize::from(plan.ch)) {
                let p = at as f64 * stem_ratio;
                let i = (p as usize).min(len - 1) - first;
                let j = (i + 1).min(left.len() - 1);
                let t = p.fract() as f32;
                let vocal = [left[i] * (1.0 - t) + left[j] * t, right[i] * (1.0 - t) + right[j] * t];
                let part = match w.part {
                    Part::Vocals => vocal,
                    Part::Instrumental => {
                        let p = at as f64 * src_ratio;
                        let i = (p as usize).min(src_len - 1);
                        let j = (i + 1).min(src_len - 1);
                        let t = p.fract() as f32;
                        std::array::from_fn(|c| {
                            audio.samples[2 * i + c] * (1.0 - t) + audio.samples[2 * j + c] * t - vocal[c]
                        })
                    }
                };
                if plan.ch == 1 {
                    output[0] = sample(0.5 * (part[0] + part[1]));
                } else {
                    output[0] = sample(part[0]);
                    output[1] = sample(part[1]);
                }
                at += 1;
            }
        }
        Ok(())
    })?;
    check(cancel)?;
    if source != SourceStamp::read(&w.path)? || crate::stems::cache_path(&w.path, w.model) != w.cache {
        return Err("The original file changed during export.".into());
    }
    let verify = probe_source(&file.path().to_string_lossy(), 0).ok_or("The encoded file cannot be read.")?;
    if verify.rate != plan.rate || verify.ch != plan.ch {
        return Err("The encoded file has unexpected audio properties.".into());
    }
    let shift = lead as f64 / f64::from(plan.rate);
    Ok(Staged {
        file,
        dir,
        plan,
        shift,
        source,
    })
}
