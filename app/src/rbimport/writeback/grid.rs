//! Patch a desktop analysis grid in its existing PMAI container. A whole-grid
//! nudge shifts each DAT timestamp and clears the measured 0x02000002 EXT mirror.
//! The older 0x01000002 mirror uses the signed PQTZ offset and keeps its fractions.
//! Rebuilding a constant tempo additionally requires a known, cue-free file set
//! and an unambiguous last beat from its detailed waveform. Entries derived from
//! beat indices (e.g. phrases) must never be copied across a reindexing.
use super::storage::FileChange;
use crate::export::anlz;
use crate::model::BeatGrid;
use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{ErrorKind, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const FRACTIONAL_PQT2: u32 = 0x0100_0002;
const DESKTOP_PQT2: u32 = 0x0200_0002;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridSnapshot {
    pub grid: BeatGrid,
    dat_path: PathBuf,
    dat_sha256: [u8; 32],
    ext_sha256: Option<[u8; 32]>,
    ex2_sha256: Option<[u8; 32]>,
    additional_sha256: Vec<(String, [u8; 32])>,
    has_embedding: bool,
    pqt2_revision: Option<u32>,
}

impl GridSnapshot {
    /// Tempo changes and bar rotations have not been verified for these profiles.
    pub(super) fn ensure_timing_editable(&self) -> Result<()> {
        ensure!(
            !self.has_embedding,
            "The native .3EX embedding contains timing metadata. Use rekordbox for this timing edit"
        );
        ensure!(
            self.pqt2_revision != Some(DESKTOP_PQT2),
            "Only a whole-grid timing shift is supported for native PQT2 revision 0x02000002"
        );
        Ok(())
    }

    pub fn dat_sha256(&self) -> [u8; 32] {
        self.dat_sha256
    }
}

pub(super) fn analysis_relative_path(input: &str) -> Result<PathBuf> {
    ensure!(input.len() <= 32_768, "Native analysis path is too long");
    let path = input.strip_prefix('/').unwrap_or(input);
    let parts: Vec<_> = path.split('/').collect();
    ensure!(
        parts.len() >= 3
            && parts[0].eq_ignore_ascii_case("PIONEER")
            && parts[1].eq_ignore_ascii_case("USBANLZ"),
        "Expected a PIONEER/USBANLZ desktop analysis path"
    );
    for part in &parts[..parts.len() - 1] {
        ensure!(
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "Unsupported analysis directory in the desktop path"
        );
    }
    let name = parts.last().unwrap();
    let stem = name
        .strip_suffix(".DAT")
        .and_then(|s| s.strip_prefix("ANLZ"));
    ensure!(
        stem.is_some_and(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit())),
        "Expected an ANLZxxxx.DAT analysis file"
    );
    Ok(parts.iter().collect())
}

fn link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions as well as ordinary symbolic links.
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn analysis_directory(share: &Path, relative: &Path) -> Result<PathBuf> {
    let share_meta = fs::symlink_metadata(share)?;
    ensure!(
        share_meta.is_dir() && !link(&share_meta),
        "Linked analysis share directory"
    );
    let mut path = share.to_path_buf();
    for component in relative.parent().unwrap().components() {
        path.push(component);
        let meta = fs::symlink_metadata(&path).context("Opening the analysis directory")?;
        ensure!(
            meta.is_dir() && !link(&meta),
            "Linked or unavailable analysis directory"
        );
    }
    Ok(path)
}

fn checked_bytes(path: &Path, meta: fs::Metadata, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        meta.is_file() && !link(&meta) && meta.len() <= limit,
        "Linked, oversized or irregular analysis file"
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == meta.len(),
        "Analysis file changed while reading"
    );
    Ok(bytes)
}

/// Read bounded PMAI bytes. Missing companions are optional; unreadable files
/// and links (including dangling links) never count as missing evidence.
pub(super) fn read_companion(
    share: &Path,
    analysis_path: &str,
    extension: &str,
) -> Result<Option<Vec<u8>>> {
    ensure!(
        !extension.is_empty()
            && extension.len() <= 32
            && extension.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Invalid analysis companion extension"
    );
    let relative = analysis_relative_path(analysis_path)?.with_extension(extension);
    let mut path = analysis_directory(share, &relative)?;
    // Windows generally ignores extension case, Linux distinguishes it. An equally
    // named differently cased file must never be mistaken for an absent companion.
    let name = relative.file_name().unwrap();
    let mut candidates = 0;
    for (index, entry) in fs::read_dir(&path)?.enumerate() {
        ensure!(index < 4096, "Too many files in the analysis directory");
        if entry?.file_name().eq_ignore_ascii_case(name) {
            candidates += 1;
        }
    }
    ensure!(
        candidates <= 1,
        "Ambiguous analysis filenames differing only in case"
    );
    path.push(name);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound && candidates == 0 => return Ok(None),
        Err(error) => return Err(error).context("Opening analysis file with its recorded name"),
    };
    checked_bytes(&path, meta, MAX_BYTES).map(Some)
}

pub fn read_sidecar(share: &Path, analysis_path: &str, extension: &str) -> Result<Option<Vec<u8>>> {
    ensure!(
        matches!(extension, "DAT" | "EXT" | "2EX"),
        "Unknown standard analysis companion"
    );
    let Some(bytes) = read_companion(share, analysis_path, extension)? else {
        return Ok(None);
    };
    anlz::validate(&bytes)
        .map_err(|reason| anyhow!("Invalid {extension} analysis file: {reason}"))?;
    Ok(Some(bytes))
}

pub(super) struct AdditionalSidecar {
    pub extension: String,
    pub bytes: Vec<u8>,
    pub is_embedding: bool,
}

/// Inspect all same-stem sibling analysis files using their contents. A .3EX
/// can itself be PMAI. Exact-profile MessagePack embeddings are recognized
/// separately because they also carry native timing metadata.
pub(super) fn additional_sidecars(
    share: &Path,
    analysis_path: &str,
) -> Result<Vec<AdditionalSidecar>> {
    let relative = analysis_relative_path(analysis_path)?;
    let stem = relative.file_stem().context("Missing analysis stem")?;
    let directory = analysis_directory(share, &relative)?;
    let mut result: Vec<AdditionalSidecar> = Vec::new();
    let mut budget = MAX_BYTES;
    for (i, entry) in fs::read_dir(directory)?.enumerate() {
        ensure!(i < 4096, "Too many files in the analysis directory");
        let path = entry?.path();
        if !path
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case(stem))
        {
            continue;
        }
        let Some(extension) = path.extension() else {
            continue;
        };
        let extension = extension
            .to_str()
            .context("Non-Unicode analysis file extension")?
            .to_ascii_uppercase();
        if matches!(extension.as_str(), "DAT" | "EXT" | "2EX") {
            continue;
        }
        ensure!(
            result.len() < 32 && !result.iter().any(|s| s.extension == extension),
            "Too many or ambiguous additional analysis companions"
        );
        let bytes = checked_bytes(&path, fs::symlink_metadata(&path)?, budget)?;
        budget -= bytes.len() as u64;
        let is_embedding =
            extension == "3EX" && !bytes.starts_with(b"PMAI") && super::embedding::is_known(&bytes);
        if !is_embedding {
            anlz::validate(&bytes).map_err(|reason| {
                anyhow!("Unsupported or invalid .{extension} companion: {reason}")
            })?;
        }
        result.push(AdditionalSidecar {
            extension,
            bytes,
            is_embedding,
        });
    }
    result.sort_by(|a, b| a.extension.cmp(&b.extension));
    Ok(result)
}

struct NativeGrid {
    state: GridSnapshot,
    dat: Vec<u8>,
    ext: Option<Vec<u8>>,
    pqtz: Range<usize>,
    pqt2: Option<Range<usize>>,
    retime_frames: Result<u32, String>,
}

// read_sidecar validates every section's framing before sections are traversed.
fn sections(data: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut offset = 28;
    std::iter::from_fn(move || {
        if offset == data.len() {
            return None;
        }
        let start = offset;
        offset += u32::from_be_bytes(data[start + 8..start + 12].try_into().unwrap()) as usize;
        Some(start..offset)
    })
}

fn grid_section(bytes: &[u8], expected: &[u8]) -> Result<Option<Range<usize>>> {
    let mut found = None;
    for range in sections(bytes) {
        let tag = &bytes[range.start..range.start + 4];
        if tag.starts_with(b"PQT") {
            ensure!(
                tag == expected && found.is_none(),
                "Unknown, misplaced or repeated native beat-grid section"
            );
            found = Some(range);
        }
    }
    Ok(found)
}

fn path_section(bytes: &[u8]) -> Result<Option<&[u8]>> {
    let mut found = None;
    for range in sections(bytes) {
        if bytes[range.clone()].starts_with(b"PPTH") {
            ensure!(
                found.is_none() && anlz::parse_ppth(&bytes[range.clone()]).is_some(),
                "Invalid analysis media path"
            );
            found = Some(&bytes[range]);
        }
    }
    Ok(found)
}

fn native_grid(
    share: &Path,
    analysis_path: &str,
    owner: Option<(&str, Option<[u8; 32]>)>,
) -> Result<NativeGrid> {
    let dat = read_sidecar(share, analysis_path, "DAT")?.context("Missing analysis DAT file")?;
    let pqtz = grid_section(&dat, b"PQTZ")?.context("No native PQTZ beat grid")?;
    let raw = &dat[pqtz.clone()];
    ensure!(
        raw.get(4..8) == Some(&24u32.to_be_bytes()) && raw.get(12..18) == Some(&[0, 0, 0, 0, 0, 8]),
        "Unknown native PQTZ header revision"
    );
    ensure!(
        raw.len() <= 24 + 1_000_000 * 8,
        "Native beat grid exceeds the pilot limit"
    );
    // Empty, explicitly counted PQTZ sections are common in both desktop
    // catalogs (samples, for example). They hold no tempo to couple to BPM.
    let grid = if raw.len() == 24 && raw[20..24] == [0; 4] {
        BeatGrid::default()
    } else {
        crate::rbimport::parse_pqtz_grid(&dat).context("Invalid native PQTZ beat records")?
    };
    let ext = read_sidecar(share, analysis_path, "EXT")?;
    let ex2 = read_sidecar(share, analysis_path, "2EX")?;
    let path = path_section(&dat)?.context("The DAT file has no recorded media path")?;
    for companion in [ext.as_deref(), ex2.as_deref()].into_iter().flatten() {
        ensure!(
            path_section(companion)? == Some(path),
            "Analysis companions refer to different media paths"
        );
    }
    if let Some(bytes) = &ex2 {
        grid_section(bytes, b"")?;
    }
    let extra = additional_sidecars(share, analysis_path)?;
    if let (Some((id, registered)), Some(sidecar)) = (owner, extra.iter().find(|s| s.is_embedding))
    {
        match super::embedding::owner(&sidecar.bytes) {
            Some(super::embedding::Owner::Track(native_id)) => ensure!(
                native_id == id.as_bytes(),
                "Native .3EX embedding belongs to another track"
            ),
            Some(super::embedding::Owner::Unassigned) => ensure!(
                registered == Some(Sha256::digest(&sidecar.bytes).into()),
                "The .3EX embedding has no track ID; a shift needs a uniquely owned registration with matching file size and checksum"
            ),
            None => bail!("Unrecognized .3EX embedding"),
        }
    }
    for other in extra.iter().filter(|s| !s.is_embedding) {
        ensure!(
            path_section(&other.bytes)?.is_none_or(|p| p == path),
            "Additional analysis companion refers to a different track"
        );
        grid_section(&other.bytes, b"")?;
    }
    let pqt2 = match &ext {
        Some(bytes) => grid_section(bytes, b"PQT2")?,
        None => None,
    };
    let pqt2_revision = if let (Some(bytes), Some(range)) = (&ext, &pqt2) {
        let extended = &bytes[range.clone()];
        ensure!(
            extended.len() >= 56 && extended[4..8] == 56u32.to_be_bytes(),
            "Invalid native PQT2 header"
        );
        let revision = u32::from_be_bytes(extended[16..20].try_into()?);
        ensure!(
            matches!(revision, FRACTIONAL_PQT2 | DESKTOP_PQT2),
            "Unsupported PQT2 revision 0x{revision:08X}. Use rekordbox to edit timing for this track"
        );
        // Desktop edits leave a canonical 56-byte empty mirror alongside a
        // populated DAT. Otherwise the anchors, count and checksum must match it.
        let cleared = revision == DESKTOP_PQT2 && extended.len() == 56;
        let count = if cleared { 0 } else { (raw.len() - 24) / 8 };
        ensure!(
            extended.len() == 56 + count * 2,
            "PQT2 count differs from the DAT grid"
        );
        let entries = if cleared {
            Vec::new()
        } else {
            anlz::pqtz_entries(raw)
        };
        let fractions = anlz::pqt2_fractions(extended);
        let mut expected = anlz::pqt2(&entries, &fractions);
        expected[16..20].copy_from_slice(&revision.to_be_bytes());
        ensure!(
            extended == expected,
            "PQT2 header fields, beat records, checksum or fractions disagree with the DAT grid"
        );
        Some(revision)
    } else {
        None
    };
    let retime_frames = retime_evidence(&dat, ext.as_deref(), ex2.as_deref(), &extra)
        .map_err(|err| err.to_string());
    let state = GridSnapshot {
        grid,
        dat_path: analysis_relative_path(analysis_path)?,
        dat_sha256: Sha256::digest(&dat).into(),
        ext_sha256: ext.as_ref().map(|b| Sha256::digest(b).into()),
        ex2_sha256: ex2.as_ref().map(|b| Sha256::digest(b).into()),
        pqt2_revision,
        has_embedding: extra.iter().any(|s| s.is_embedding),
        additional_sha256: extra
            .into_iter()
            .map(|s| (s.extension, Sha256::digest(s.bytes).into()))
            .collect(),
    };
    Ok(NativeGrid {
        state,
        dat,
        ext,
        pqtz,
        pqt2,
        retime_frames,
    })
}

// Detailed waveforms have one frame per 1/150 s. Only use that evidence when
// the file revision and shape agree, and both possible endpoints produce the
// same beat count. This avoids inventing a duration from rounded DB seconds.
fn retime_evidence(
    dat: &[u8],
    ext: Option<&[u8]>,
    ex2: Option<&[u8]>,
    other: &[AdditionalSidecar],
) -> Result<u32> {
    let ext = ext.context("Retiming requires a paired EXT with a detailed waveform")?;
    let mut frames = None;
    let pmai = other
        .iter()
        .filter(|s| !s.is_embedding)
        .map(|s| s.bytes.as_slice());
    for bytes in [Some(dat), Some(ext), ex2]
        .into_iter()
        .flatten()
        .chain(pmai)
    {
        for range in sections(bytes) {
            let tag = &bytes[range.clone()];
            match &tag[..4] {
                b"PPTH" | b"PVBR" | b"PWAV" | b"PWV2" | b"PWV4" | b"PWV6" | b"PWV7" | b"PWVC" | b"PQTZ" | b"PQT2" => {}
                b"PVDI" => {
                    // Fixed audio-time hops carry no beat indices. Permit the
                    // measured layout only, including its exact payload length.
                    ensure!(
                        tag.len() >= 24
                            && tag[4..8] == 24u32.to_be_bytes()
                            && tag[12..20] == [0, 0, 4, 0, 0x56, 0x22, 0, 1]
                            && u32::from_be_bytes(tag[20..24].try_into()?) as usize == tag.len() - 24
                            && tag[24..].iter().all(|v| *v <= 4),
                        "Unrecognized native vocal activity layout"
                    );
                }
                b"PWV3" | b"PWV5" => {
                    let (width, revision): (usize, u32) = if &tag[..4] == b"PWV3" {
                        (1, 0x0096_0000)
                    } else {
                        (2, 0x0096_0305)
                    };
                    ensure!(
                        tag.len() >= 24
                            && tag[4..8] == 24u32.to_be_bytes()
                            && tag[12..16] == (width as u32).to_be_bytes()
                            && tag[20..24] == revision.to_be_bytes(),
                        "Unrecognized detailed waveform revision"
                    );
                    let n = u32::from_be_bytes(tag[16..20].try_into()?);
                    ensure!(
                        n > 0 && tag.len() == 24 + width * n as usize && frames.is_none_or(|prev| prev == n),
                        "Detailed waveform counts disagree or are empty"
                    );
                    frames = Some(n);
                }
                b"PCOB" => ensure!(
                    tag.len() == 24 && tag[4..8] == 24u32.to_be_bytes() && tag[18..20] == [0, 0],
                    "Retiming native analysis with cue or loop references needs review in rekordbox"
                ),
                b"PCO2" => ensure!(
                    tag.len() == 20 && tag[4..8] == 20u32.to_be_bytes() && tag[16..18] == [0, 0],
                    "Retiming native analysis with extended cue references needs review in rekordbox"
                ),
                tag => bail!(
                    "Retiming analysis containing {} needs review in rekordbox",
                    String::from_utf8_lossy(tag)
                ),
            }
        }
    }
    frames.context("Retiming requires a validated 150 Hz detailed waveform")
}

fn replace_section(file: &mut Vec<u8>, range: Range<usize>, section: &[u8]) -> Result<()> {
    let new_len = file.len() - range.len() + section.len();
    ensure!(
        new_len as u64 <= MAX_BYTES,
        "Updated analysis file exceeds size limit"
    );
    file.splice(range, section.iter().copied());
    file[8..12].copy_from_slice(&(new_len as u32).to_be_bytes());
    Ok(())
}

fn retime(mut native: NativeGrid, desired: &BeatGrid) -> Result<Vec<FileChange>> {
    let frames = native.retime_frames.map_err(anyhow::Error::msg)?;
    let old_fractions = native
        .pqt2
        .as_ref()
        .map(|r| anlz::pqt2_fractions(&native.ext.as_ref().unwrap()[r.clone()]));
    let fraction = |i: usize| old_fractions.as_ref().map_or(0, |fs| fs[i]);
    let raw = &native.dat[native.pqtz.clone()];
    let native_shift = i16::from_be_bytes(raw[18..20].try_into()?) as i64;
    let entries = anlz::pqtz_entries(raw);
    let first = entries.first().context("Empty original grid")?;
    let old_first_us = (i64::from(first.2) + native_shift) * 1000 + i64::from(fraction(0));
    let old_period_us = 6_000_000_000.0 / f64::from(first.1);
    for (index, entry) in entries.iter().enumerate() {
        let adjusted = (i64::from(entry.2) + native_shift) * 1000 + i64::from(fraction(index));
        let expected = old_first_us as f64 + index as f64 * old_period_us;
        ensure!(
            entry.1 == first.1
                && entry.0 == ((first.0 as usize - 1 + index) % 4 + 1) as u16
                && (adjusted as f64 - expected).abs() < 1100.0,
            "Original beat records vary from their nominal constant grid"
        );
    }
    let end_lower = u64::from(frames - 1) * 1_000_000 / 150;
    let end_upper = (u64::from(frames) * 1_000_000).div_ceil(150);
    let old_last = entries.last().unwrap();
    // A recorded beat at the track endpoint is valid (common for loop samples).
    // Its millisecond timestamp can round up from the final waveform frame.
    let old_last_us = ((i64::from(old_last.2) + native_shift) * 1000
        + i64::from(fraction(entries.len() - 1))) as f64;
    ensure!(
        old_first_us >= 0
            && (old_first_us as f64) < old_period_us
            && old_last_us <= end_upper as f64 + 1000.
            && old_last_us + old_period_us > end_upper as f64,
        "Original grid does not unambiguously cover the waveform"
    );
    let anchor = desired.anchors[0];
    let bpm = (anchor.bpm * 100.).round();
    ensure!(
        (40.0..=499.0).contains(&anchor.bpm)
            && (anchor.bpm * 100. - bpm).abs() < 1e-6
            && (1..=4).contains(&anchor.beat)
            && anchor.time.is_finite()
            && anchor.time >= 0.0,
        "Retiming requires a representable constant BPM, first beat and bar number"
    );
    let first_ms = (anchor.time * 1000.).round();
    ensure!(
        anchor.time < 60. / anchor.bpm && (anchor.time * 1000. - first_ms).abs() < 1e-6,
        "Set the tempo at the track's first beat with millisecond precision to write this grid"
    );
    let first_us = first_ms as u64 * 1000 + u64::from(fraction(0));
    ensure!(
        (first_us as f64) < 60_000_000. / anchor.bpm,
        "Fractional timing crosses the first beat boundary"
    );
    let bpm = bpm as u64;
    let mut beats = Vec::new();
    let mut fractions = Vec::new();
    // Integer microseconds avoid cumulative floating-point drift.
    for index in 0..=1_000_000u64 {
        let us = first_us + (index * 6_000_000_000 + bpm / 2) / bpm;
        if us > end_upper {
            break;
        }
        ensure!(
            us < end_lower,
            "Track endpoint is too close to a proposed beat to determine its inclusion"
        );
        ensure!(
            index < 1_000_000 && us / 1000 <= u64::from(u32::MAX),
            "Updated grid exceeds native file limits"
        );
        beats.push((
            ((u64::from(anchor.beat) - 1 + index) % 4 + 1) as u16,
            bpm as u16,
            (us / 1000) as u32,
        ));
        fractions.push((us % 1000) as u16);
    }
    ensure!(
        !beats.is_empty(),
        "Updated grid has no beats within the track"
    );
    let mut pqtz = raw[..24].to_vec();
    pqtz[8..12].copy_from_slice(&((24 + beats.len() * 8) as u32).to_be_bytes());
    pqtz[18..20].copy_from_slice(&0i16.to_be_bytes());
    pqtz[20..24].copy_from_slice(&(beats.len() as u32).to_be_bytes());
    for &(number, tempo, time) in &beats {
        pqtz.extend_from_slice(&number.to_be_bytes());
        pqtz.extend_from_slice(&tempo.to_be_bytes());
        pqtz.extend_from_slice(&time.to_be_bytes());
    }
    replace_section(&mut native.dat, native.pqtz, &pqtz)?;

    let reread =
        crate::rbimport::parse_pqtz_grid(&native.dat).context("Rebuilt grid is unreadable")?;
    ensure!(
        crate::app::grids_match(&reread, desired),
        "Rebuilt grid differs from requested anchors"
    );
    let path = Path::new("share").join(&native.state.dat_path);
    let mut changes = Vec::new();
    if let Some(range) = native.pqt2 {
        let mut ext = native.ext.context("Missing file for paired PQT2")?;
        replace_section(&mut ext, range, &anlz::pqt2(&beats, &fractions))?;
        changes.push(FileChange {
            relative_path: path.with_extension("EXT"),
            expected_sha256: native.state.ext_sha256.unwrap(),
            contents: ext,
        });
    }
    changes.push(FileChange {
        relative_path: path,
        expected_sha256: native.state.dat_sha256,
        contents: native.dat,
    });
    Ok(changes)
}

pub fn inspect(share: &Path, analysis_path: &str) -> Result<GridSnapshot> {
    Ok(native_grid(share, analysis_path, None)?.state)
}

/// Returns complete replacement bytes for the transaction's verified backup and
/// recovery journal. Nothing is written by this function.
pub fn plan(
    share: &Path,
    analysis_path: &str,
    baseline: &GridSnapshot,
    desired: &BeatGrid,
    owner_id: &str,
    registered_embedding: Option<[u8; 32]>,
) -> Result<Vec<FileChange>> {
    ensure!(
        !desired.has_listed_beats(),
        "A locally analyzed per-beat grid requires native reanalysis in AlphaTheta Rekordbox. Desktop write-back supports reviewed edits to existing native tempo segments."
    );
    let owner = (&baseline.grid != desired).then_some((owner_id, registered_embedding));
    let mut native = native_grid(share, analysis_path, owner)?;
    ensure!(
        &native.state == baseline,
        "Analysis files changed since the grid preview"
    );
    let source = &baseline.grid;
    if source == desired {
        return Ok(Vec::new());
    }
    ensure!(
        !desired.anchors.is_empty() && desired.anchors.len() == source.anchors.len(),
        "Grid segment insertion or removal requires native reanalysis"
    );
    if source.anchors.len() == 1 && (desired.anchors[0].bpm - source.anchors[0].bpm).abs() > 1e-9 {
        baseline.ensure_timing_editable()?;
        return retime(native, desired);
    }
    let shift = (desired.anchors[0].time - source.anchors[0].time) * 1000.;
    let rotation =
        (i32::from(desired.anchors[0].beat) - i32::from(source.anchors[0].beat)).rem_euclid(4);
    for (from, to) in source.anchors.iter().zip(&desired.anchors) {
        ensure!(
            to.time.is_finite()
                && to.time >= 0.
                && (1..=4).contains(&to.beat)
                && (to.bpm - from.bpm).abs() <= 1e-9,
            "Native multi-tempo grids support whole-grid timing and bar-phase edits"
        );
        ensure!(
            ((to.time - from.time) * 1000. - shift.round()).abs() <= 1e-6
                && (i32::from(to.beat) - i32::from(from.beat)).rem_euclid(4) == rotation,
            "The grid edit is not a uniform whole-millisecond shift or bar rotation"
        );
    }
    if shift.round() == 0. && rotation == 0 {
        return Ok(Vec::new());
    }
    let position = native.pqtz.start;
    let shift_records = rotation == 0 && baseline.pqt2_revision != Some(FRACTIONAL_PQT2);
    if shift_records {
        // The two desktop captures moved the stored millisecond timestamps,
        // preserving BPM, beat-in-bar and the existing header offset.
        let delta = shift.round();
        ensure!(
            delta.abs() <= f64::from(u32::MAX),
            "Shift exceeds native timestamp range"
        );
        for entry in native.dat[position + 24..native.pqtz.end].chunks_exact_mut(8) {
            let ms = u32::from_be_bytes(entry[4..8].try_into()?);
            let shifted = u32::try_from(i64::from(ms) + delta as i64)
                .context("Shift crosses the native timestamp range")?;
            entry[4..8].copy_from_slice(&shifted.to_be_bytes());
        }
    } else {
        baseline.ensure_timing_editable()?;
        let old_shift = i16::from_be_bytes(native.dat[position + 18..position + 20].try_into()?);
        let total = f64::from(old_shift) + shift.round();
        ensure!(
            (f64::from(i16::MIN)..=f64::from(i16::MAX)).contains(&total),
            "Offset exceeds the native signed millisecond field"
        );
        native.dat[position + 18..position + 20].copy_from_slice(&(total as i16).to_be_bytes());
    }
    if rotation != 0 {
        for entry in native.dat[position + 24..native.pqtz.end].chunks_exact_mut(8) {
            let old = u16::from_be_bytes([entry[0], entry[1]]);
            let bar = (i32::from(old) - 1 + rotation).rem_euclid(4) as u16 + 1;
            entry[..2].copy_from_slice(&bar.to_be_bytes());
        }
    }
    let reread = crate::rbimport::parse_pqtz_grid(&native.dat)
        .context("Edited grid has unsupported preroll or invalid beats")?;
    ensure!(
        crate::app::grids_match(&reread, desired),
        "Edited native grid does not round-trip to the requested anchors"
    );
    let dat_path = Path::new("share").join(&native.state.dat_path);
    let mut changes = vec![];
    if let (Some(mut ext), Some(range), Some(digest)) =
        (native.ext, native.pqt2, native.state.ext_sha256)
    {
        let replacement = if shift_records && range.len() > 56 {
            let mut empty = anlz::pqt2(&[], &[]);
            empty[16..20].copy_from_slice(&DESKTOP_PQT2.to_be_bytes());
            Some(empty)
        } else if rotation != 0 {
            let entries = anlz::pqtz_entries(&native.dat[native.pqtz]);
            let fractions = anlz::pqt2_fractions(&ext[range.clone()]);
            Some(anlz::pqt2(&entries, &fractions))
        } else {
            None
        };
        if let Some(section) = replacement {
            replace_section(&mut ext, range, &section)?;
            changes.push(FileChange {
                relative_path: dat_path.with_extension("EXT"),
                expected_sha256: digest,
                contents: ext,
            });
        }
    }
    changes.push(FileChange {
        relative_path: dat_path,
        expected_sha256: baseline.dat_sha256,
        contents: native.dat,
    });
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Beat, Waveform};

    const LOCATION: &str = "/PIONEER/USBANLZ/P001/00000001/ANLZ0000.DAT";

    fn plan(
        share: &Path,
        path: &str,
        snap: &GridSnapshot,
        desired: &BeatGrid,
    ) -> Result<Vec<FileChange>> {
        super::plan(share, path, snap, desired, "123", None)
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let share = root.path().join("share");
        let dat = share.join(LOCATION.trim_start_matches('/'));
        fs::create_dir_all(dat.parent().unwrap()).unwrap();
        let beats = (0..20)
            .map(|index| Beat {
                index,
                time: 0.10025 + index as f64 * 0.5,
                bpm: 120.,
                beat: (index % 4 + 1) as u8,
            })
            .collect::<Vec<_>>();
        let wave = Waveform {
            frames: vec![[0; 8]; 1500],
        };
        let files = anlz::build("/music/grid.wav", &beats, &wave, &[], 0, 44100);
        fs::write(&dat, files.dat).unwrap();
        fs::write(dat.with_extension("EXT"), files.ext).unwrap();
        fs::write(dat.with_extension("2EX"), files.ex2).unwrap();
        (root, share, dat)
    }

    #[test]
    fn per_beat_grid_cannot_be_mistaken_for_a_native_segment_edit() -> Result<()> {
        let (_root, share, dat) = fixture();
        let baseline = inspect(&share, LOCATION)?;
        // Even an identical first beat and tempo do not prove identical native beat rows.
        let per_beat = BeatGrid::from_listed(baseline.grid.anchors.clone());
        let bytes = fs::read(&dat)?;
        let err = plan(&share, LOCATION, &baseline, &per_beat).err().expect("expected a blocked plan");
        assert!(err.to_string().contains("per-beat grid"), "{err}");
        assert_eq!(fs::read(&dat)?, bytes);
        Ok(())
    }

    #[test]
    fn desktop_offset_changes_only_signed_header_and_keeps_companions() -> Result<()> {
        let (_root, share, dat) = fixture();
        let baseline = inspect(&share, LOCATION)?;
        let before = fs::read(&dat)?;
        let ext_before = fs::read(dat.with_extension("EXT"))?;
        let mut desired = baseline.grid.clone();
        desired.anchors[0].time -= 0.021;
        let changes = plan(&share, LOCATION, &baseline, &desired)?;
        assert_eq!(changes.len(), 1);
        let edit = &changes[0];
        let offset = grid_section(&before, b"PQTZ")?.unwrap().start + 18;
        assert_eq!(
            edit.expected_sha256,
            <[u8; 32]>::from(Sha256::digest(&before))
        );
        assert_eq!(edit.contents[..offset], before[..offset]);
        assert_eq!(edit.contents[offset..offset + 2], (-21i16).to_be_bytes());
        assert_eq!(edit.contents[offset + 2..], before[offset + 2..]);
        assert_eq!(fs::read(dat)?, before);
        assert_eq!(
            fs::read(share.join(&baseline.dat_path).with_extension("EXT"))?,
            ext_before
        );
        Ok(())
    }

    #[test]
    fn rotating_downbeat_rebuilds_only_paired_extended_metadata() -> Result<()> {
        let (root, share, dat) = fixture();
        let baseline = inspect(&share, LOCATION)?;
        let before = fs::read(dat.with_extension("EXT"))?;
        let section = grid_section(&before, b"PQT2")?.unwrap();
        let mut desired = baseline.grid.clone();
        desired.anchors[0].beat = 4;
        let changes = plan(&share, LOCATION, &baseline, &desired)?;
        assert_eq!(changes.len(), 2);
        for patch in changes {
            if patch.relative_path.extension().unwrap() == "EXT" {
                assert_eq!(before[..section.start], patch.contents[..section.start]);
                assert_eq!(
                    before[section.start + 56..],
                    patch.contents[section.start + 56..]
                );
            }
            fs::write(root.path().join(patch.relative_path), patch.contents)?;
        }
        let after = inspect(&share, LOCATION)?;
        assert_eq!(after.grid, desired);
        assert!(plan(&share, LOCATION, &after, &desired)?.is_empty());
        assert!(plan(&share, LOCATION, &baseline, &desired).is_err());
        Ok(())
    }

    #[test]
    fn unknown_or_inconsistent_companions_and_nonuniform_edits_are_rejected() -> Result<()> {
        let (_root, share, dat) = fixture();
        let before = inspect(&share, LOCATION)?;
        let mut desired = before.grid.clone();
        desired.anchors[0].time += 0.0004;
        assert!(plan(&share, LOCATION, &before, &desired).is_err());
        desired.anchors[0].bpm = 130.;
        assert!(plan(&share, LOCATION, &before, &desired).is_err());
        let ext = dat.with_extension("EXT");
        let bytes = fs::read(&ext)?;
        let offset = grid_section(&bytes, b"PQT2")?.unwrap().start;
        for field in [16, 24, 40, 44, 56] {
            let mut changed = bytes.clone();
            changed[offset + field] ^= 0x80;
            fs::write(&ext, changed)?;
            assert!(inspect(&share, LOCATION).is_err());
        }
        assert!(inspect(&share, "/PIONEER/USBANLZ/../../ANLZ0000.DAT").is_err());
        assert!(inspect(&share, "//PIONEER/USBANLZ/P001/ANLZ0000.DAT").is_err());
        Ok(())
    }

    #[test]
    fn unrecognized_pqt2_revision_is_reported_without_regenerating_its_data() -> Result<()> {
        let (_root, share, dat) = fixture();
        let snap = inspect(&share, LOCATION)?;
        let mut desired = snap.grid.clone();
        desired.anchors[0].beat = 2;
        let ext = dat.with_extension("EXT");
        let mut bytes = fs::read(&ext)?;
        let offset = grid_section(&bytes, b"PQT2")?.unwrap().start + 16;
        for revision in [0x0300_0002u32, 0x1234_abcd] {
            bytes[offset..offset + 4].copy_from_slice(&revision.to_be_bytes());
            fs::write(&ext, &bytes)?;
            let error = plan(&share, LOCATION, &snap, &desired).unwrap_err();
            assert!(error
                .to_string()
                .contains(&format!("Unsupported PQT2 revision 0x{revision:08X}")));
            assert_eq!(fs::read(&ext)?, bytes);
        }
        Ok(())
    }

    #[test]
    fn desktop_nudge_matches_native_timestamps_and_empties_only_the_mirror() -> Result<()> {
        let (root, share, dat) = fixture();
        let ext = dat.with_extension("EXT");
        let mut original_ext = fs::read(&ext)?;
        let mirror = grid_section(&original_ext, b"PQT2")?.unwrap();
        original_ext[mirror.start + 16..mirror.start + 20]
            .copy_from_slice(&DESKTOP_PQT2.to_be_bytes());
        fs::write(&ext, &original_ext)?;
        let original = fs::read(&dat)?;
        let section = grid_section(&original, b"PQTZ")?.unwrap();
        let mut expected = original.clone();
        for entry in expected[section.start + 24..section.end].chunks_exact_mut(8) {
            let ms = u32::from_be_bytes(entry[4..8].try_into()?);
            entry[4..8].copy_from_slice(&(ms + 1).to_be_bytes());
        }
        let mut expected_ext = original_ext.clone();
        let mut header = original_ext[mirror.start..mirror.start + 56].to_vec();
        header[8..12].copy_from_slice(&56u32.to_be_bytes());
        header[24..].fill(0);
        replace_section(&mut expected_ext, mirror.clone(), &header)?;

        let baseline = inspect(&share, LOCATION)?;
        let mut desired = baseline.grid.clone();
        desired.anchors[0].time += 0.001;
        let changes = plan(&share, LOCATION, &baseline, &desired)?;
        assert_eq!(changes.len(), 2);
        for patch in changes {
            fs::write(root.path().join(patch.relative_path), patch.contents)?;
        }
        assert_eq!(fs::read(&dat)?, expected);
        assert_eq!(fs::read(&ext)?, expected_ext);
        let cleared = inspect(&share, LOCATION)?;
        assert!(crate::app::grids_match(&cleared.grid, &desired));
        let back = plan(&share, LOCATION, &cleared, &baseline.grid)?;
        assert_eq!(back.len(), 1, "A second nudge keeps the cleared mirror");
        assert_eq!(back[0].contents, original);
        for change in [
            BeatGrid::constant(0.100, 120., 2),
            BeatGrid::constant(0.100, 130., 1),
        ] {
            assert!(plan(&share, LOCATION, &cleared, &change).is_err());
        }
        let mut corrupt = expected_ext;
        corrupt[mirror.start + 44] = 1;
        fs::write(ext, corrupt)?;
        assert!(inspect(&share, LOCATION).is_err());
        Ok(())
    }

    #[test]
    fn nudge_without_mirror_keeps_per_beat_jitter_and_pqtz_offset() -> Result<()> {
        let (_root, share, dat) = fixture();
        let ext = dat.with_extension("EXT");
        let mut bytes = fs::read(&ext)?;
        let mirror = grid_section(&bytes, b"PQT2")?.unwrap();
        replace_section(&mut bytes, mirror, &[])?;
        fs::write(&ext, &bytes)?;
        let mut original = fs::read(&dat)?;
        let section = grid_section(&original, b"PQTZ")?.unwrap();
        original[section.start + 18..section.start + 20].copy_from_slice(&5i16.to_be_bytes());
        // Give interior beats distinct tempos and timing offsets that cannot be
        // reconstructed by regenerating a regular grid from its first anchor.
        for (i, entry) in original[section.start + 24..section.end]
            .chunks_exact_mut(8)
            .enumerate()
        {
            let ms = u32::from_be_bytes(entry[4..8].try_into()?);
            entry[4..8].copy_from_slice(&(ms + (i % 3) as u32).to_be_bytes());
            entry[2..4].copy_from_slice(&(12000 + (i % 2) as u16).to_be_bytes());
        }
        fs::write(&dat, &original)?;
        let baseline = inspect(&share, LOCATION)?;
        let mut desired = baseline.grid.clone();
        for anchor in &mut desired.anchors {
            anchor.time -= 0.001;
        }
        let patches = plan(&share, LOCATION, &baseline, &desired)?;
        assert_eq!(patches.len(), 1);
        let raw = &patches[0].contents;
        assert_eq!(raw[..section.start + 24], original[..section.start + 24]);
        assert_eq!(raw[section.end..], original[section.end..]);
        let before = anlz::pqtz_entries(&original[section.clone()]);
        let after = anlz::pqtz_entries(&raw[section]);
        for ((bar, bpm, ms), entry) in before.into_iter().zip(after) {
            assert_eq!((bar, bpm, ms - 1), entry);
        }
        assert_eq!(fs::read(ext)?, bytes);
        Ok(())
    }

    #[test]
    fn nudge_rejects_overflow_at_either_end_of_raw_timestamps() -> Result<()> {
        let (_root, share, dat) = fixture();
        fs::remove_file(dat.with_extension("EXT"))?;
        let mut bytes = fs::read(&dat)?;
        let section = grid_section(&bytes, b"PQTZ")?.unwrap();
        // Both requested first-anchor times below fit in the model. Only checking
        // every raw timestamp catches the negative first or overflowing last beat.
        bytes[section.start + 18..section.start + 20].copy_from_slice(&5i16.to_be_bytes());
        bytes[section.start + 28..section.start + 32].copy_from_slice(&0u32.to_be_bytes());
        bytes[section.end - 4..section.end].copy_from_slice(&u32::MAX.to_be_bytes());
        fs::write(&dat, &bytes)?;
        let baseline = inspect(&share, LOCATION)?;
        for delta in [-0.001, 0.001] {
            let mut desired = baseline.grid.clone();
            desired.anchors[0].time += delta;
            let error = plan(&share, LOCATION, &baseline, &desired).unwrap_err();
            assert!(error.to_string().contains("timestamp range"), "{error:#}");
        }
        assert_eq!(fs::read(dat)?, bytes);
        Ok(())
    }

    #[test]
    fn constant_tempo_rebuild_adjusts_count_and_fractional_timing() -> Result<()> {
        let (root, share, dat) = fixture();
        let before = inspect(&share, LOCATION)?;
        let ambiguous = BeatGrid::constant(0.398, 125., 1);
        let err = plan(&share, LOCATION, &before, &ambiguous).unwrap_err();
        assert!(err.to_string().contains("endpoint"), "{err}");
        let ext_path = dat.with_extension("EXT");
        let ext = fs::read(&ext_path)?;
        let old_range = grid_section(&ext, b"PQT2")?.unwrap();
        let original_first_fraction = anlz::pqt2_fractions(&ext[old_range.clone()])[0];
        let desired = BeatGrid::constant(before.grid.anchors[0].time, 127.0, 1);
        let changes = plan(&share, LOCATION, &before, &desired)?;
        assert_eq!(changes.len(), 2);
        for patch in changes {
            fs::write(root.path().join(patch.relative_path), patch.contents)?;
        }
        let after = inspect(&share, LOCATION)?;
        assert_eq!(after.grid, desired);
        let rewritten = fs::read(&ext_path)?;
        let new_range = grid_section(&rewritten, b"PQT2")?.unwrap();
        assert_eq!(ext[12..old_range.start], rewritten[12..new_range.start]);
        assert_eq!(ext[old_range.end..], rewritten[new_range.end..]);
        let fractions = anlz::pqt2_fractions(&rewritten[new_range]);
        assert_eq!(fractions.len(), 21);
        assert_eq!(fractions[0], original_first_fraction);
        let bytes = fs::read(&dat)?;
        let native_beats = anlz::pqtz_entries(&bytes[grid_section(&bytes, b"PQTZ")?.unwrap()]);
        for (i, (&(bar, bpm, ms), fraction)) in native_beats.iter().zip(fractions).enumerate() {
            assert_eq!((bar, bpm), ((i % 4 + 1) as u16, 12700));
            let expected_us =
                100000. + f64::from(original_first_fraction) + i as f64 * 60_000_000. / 127.;
            assert!((f64::from(ms) * 1000. + f64::from(fraction) - expected_us).abs() <= 0.5);
        }
        Ok(())
    }

    #[test]
    fn retiming_refuses_phrases_and_unverified_native_companions() -> Result<()> {
        let (_root, share, dat) = fixture();
        let third = dat.with_extension("3EX");
        fs::write(&third, b"unknown analysis")?;
        assert!(inspect(&share, LOCATION).is_err());
        fs::remove_file(third)?;
        let desired = BeatGrid::constant(0.100, 127., 1);
        let ext = dat.with_extension("EXT");
        let mut bytes = fs::read(&ext)?;
        bytes.extend_from_slice(b"PSSI");
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(&12u32.to_be_bytes());
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_be_bytes());
        fs::write(&ext, bytes)?;
        let before = inspect(&share, LOCATION)?;
        assert!(plan(&share, LOCATION, &before, &desired).is_err());
        Ok(())
    }

    fn graft(path: &Path, section: &[u8]) -> Result<()> {
        let mut data = fs::read(path)?;
        let end = data.len();
        replace_section(&mut data, end..end, section)?;
        fs::write(path, data)?;
        Ok(())
    }

    #[test]
    fn extended_grid_is_optional_and_empty_native_grid_is_not_corrupt() -> Result<()> {
        let (root, share, dat) = fixture();
        let ext = dat.with_extension("EXT");
        let mut bytes = fs::read(&ext)?;
        let mirror = grid_section(&bytes, b"PQT2")?.unwrap();
        replace_section(&mut bytes, mirror, &[])?;
        fs::write(&ext, &bytes)?;
        let before = inspect(&share, LOCATION)?;
        let wanted = BeatGrid::constant(0.1, 127., 1);
        let mut changes = plan(&share, LOCATION, &before, &wanted)?;
        assert_eq!(changes.len(), 1);
        let change = changes.pop().unwrap();
        assert_eq!(root.path().join(change.relative_path), dat);
        fs::write(&dat, change.contents)?;
        assert_eq!(inspect(&share, LOCATION)?.grid, wanted);
        assert_eq!(
            fs::read(&ext)?,
            bytes,
            "Rekordbox stored no extended grid for this track"
        );
        let mut data = fs::read(&dat)?;
        let section = grid_section(&data, b"PQTZ")?.unwrap();
        let mut header = data[section.start..section.start + 24].to_vec();
        header[8..12].copy_from_slice(&24u32.to_be_bytes());
        header[20..24].copy_from_slice(&0u32.to_be_bytes());
        replace_section(&mut data, section.clone(), &header)?;
        fs::write(&dat, &data)?;
        let empty = inspect(&share, LOCATION)?;
        assert!(empty.grid.is_empty());
        assert!(plan(&share, LOCATION, &empty, &wanted).is_err());
        data[section.start + 23] = 1;
        fs::write(dat, data)?;
        assert!(inspect(&share, LOCATION).is_err());
        Ok(())
    }

    #[test]
    fn extra_containers_and_fixed_audio_time_vocals_follow_their_contents() -> Result<()> {
        let (_root, share, dat) = fixture();
        let third = dat.with_extension("3EX");
        let mut bytes = fs::read(dat.with_extension("2EX"))?;
        let known = anlz::pvdi(&[0, 1, 4, 1]);
        let end = bytes.len();
        replace_section(&mut bytes, end..end, &known)?;
        fs::write(&third, &bytes)?;
        let start = inspect(&share, LOCATION)?;
        let wanted = BeatGrid::constant(0.1, 127., 1);
        assert_eq!(plan(&share, LOCATION, &start, &wanted)?.len(), 2);
        assert_eq!(fs::read(&third)?, bytes);
        let unknown = [
            b"PNEW".as_slice(),
            &12u32.to_be_bytes(),
            &12u32.to_be_bytes(),
        ]
        .concat();
        graft(&third, &unknown)?;
        assert!(
            plan(&share, LOCATION, &start, &wanted).is_err(),
            "New companion contents invalidate an existing review"
        );
        let changed = inspect(&share, LOCATION)?;
        assert!(
            plan(&share, LOCATION, &changed, &wanted).is_err(),
            "Unknown semantic tags exclude beat reindexing"
        );
        let shifted = BeatGrid::constant(0.101, 120., 1);
        let opaque = fs::read(&third)?;
        let offset = plan(&share, LOCATION, &changed, &shifted)?;
        assert_eq!(offset.len(), 1);
        assert_eq!(fs::read(&third)?, opaque);
        let duplicate = [
            b"PQT9".as_slice(),
            &12u32.to_be_bytes(),
            &12u32.to_be_bytes(),
        ]
        .concat();
        graft(&third, &duplicate)?;
        assert!(
            inspect(&share, LOCATION).is_err(),
            "A future duplicate grid requires a coordinated writer"
        );
        fs::write(&third, b"PMAI...")?;
        assert!(inspect(&share, LOCATION).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "set RECORDBOX_TEST_PYREKORDBOX and RECORDBOX_TEST_RBOX to the pinned public fixtures"]
    fn published_v6_and_v7_analysis_companions_are_checked_and_preserved() -> Result<()> {
        let env = |key| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .context("Missing public fixture checkout")
        };
        let v6 = env("RECORDBOX_TEST_PYREKORDBOX")?.join(".testdata/rekordbox 6/backup/share");
        let v7 = env("RECORDBOX_TEST_RBOX")?.join(".testdata/RBv7/demo/rekordbox/share");
        for (share, version) in [(v6, 6), (v7, 7)] {
            let mut grids = 0;
            let mut empty = 0;
            let mut embeddings = 0;
            let mut editable_grids = 0;
            let mut protected_grids = 0;
            for f in walkdir::WalkDir::new(&share) {
                let entry = f?;
                let path = entry.path();
                if path.file_name().is_none_or(|name| name != "ANLZ0000.DAT") {
                    continue;
                }
                let native_path = path
                    .strip_prefix(&share)?
                    .to_str()
                    .context("Invalid public path")?
                    .replace('\\', "/");
                let originals = ["DAT", "EXT", "2EX", "3EX"]
                    .into_iter()
                    .map(|ext| {
                        let file = path.with_extension(ext);
                        if !file.exists() {
                            return Ok(None);
                        }
                        let bytes = fs::read(&file)?;
                        Ok(Some((file, <[u8; 32]>::from(Sha256::digest(bytes)))))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let snap = inspect(&share, &native_path)?;
                embeddings += usize::from(snap.has_embedding);
                assert!(plan(&share, &native_path, &snap, &snap.grid)?.is_empty());
                if snap.has_embedding {
                    assert!(
                        snap.ensure_timing_editable().is_err(),
                        "Even scalar BPM needs its mirror"
                    );
                }
                if snap.grid.is_empty() {
                    empty += 1;
                } else {
                    grids += 1;
                    let mut shift = snap.grid.clone();
                    for a in &mut shift.anchors {
                        a.time += 0.001;
                    }
                    let embedded_bytes = fs::read(path.with_extension("3EX")).ok();
                    let owner = match embedded_bytes
                        .as_deref()
                        .and_then(super::super::embedding::owner)
                    {
                        Some(super::super::embedding::Owner::Track(id)) => std::str::from_utf8(id)?,
                        _ => "",
                    };
                    if snap.has_embedding {
                        let mut rotate = snap.grid.clone();
                        for a in &mut rotate.anchors {
                            a.beat = a.beat % 4 + 1;
                        }
                        let mut tempo = snap.grid.clone();
                        tempo.anchors[0].bpm += 1.;
                        for requested in [&rotate, &tempo] {
                            assert!(super::plan(
                                &share,
                                &native_path,
                                &snap,
                                requested,
                                owner,
                                None
                            )
                            .is_err());
                        }
                    }
                    let files = super::plan(&share, &native_path, &snap, &shift, owner, None);
                    if snap.has_embedding && snap.pqt2_revision == Some(FRACTIONAL_PQT2) {
                        assert!(files.unwrap_err().to_string().contains(".3EX embedding"));
                        protected_grids += 1;
                    } else {
                        let files = files?;
                        assert_eq!(files.len(), 1);
                        let before = fs::read(path)?;
                        let range = grid_section(&before, b"PQTZ")?.unwrap();
                        if snap.pqt2_revision == Some(FRACTIONAL_PQT2) {
                            assert_eq!(
                                &files[0].contents[..range.start + 18],
                                &before[..range.start + 18]
                            );
                            assert_eq!(
                                &files[0].contents[range.start + 20..],
                                &before[range.start + 20..]
                            );
                        } else {
                            let original = anlz::pqtz_entries(&before[range.clone()]);
                            let shifted = anlz::pqtz_entries(&files[0].contents[range]);
                            for ((bar, bpm, t), after) in original.into_iter().zip(shifted) {
                                assert_eq!(after, (bar, bpm, t + 1));
                            }
                        }
                        editable_grids += 1;
                    }
                }
                for (file, hash) in originals.into_iter().flatten() {
                    assert_eq!(hash, <[u8; 32]>::from(Sha256::digest(fs::read(file)?)));
                }
            }
            eprintln!("Public v{version}: {grids} populated grids, {empty} empty, {embeddings} embeddings, {protected_grids} populated grids protected, {editable_grids} phase edits");
            let expected = if version == 6 {
                (2, 4, 0, 0, 2)
            } else {
                (10, 22, 29, 2, 8)
            };
            assert_eq!(
                (grids, empty, embeddings, protected_grids, editable_grids),
                expected
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn linked_analysis_companion_is_never_used() -> Result<()> {
        let (root, share, dat) = fixture();
        let ext = dat.with_extension("EXT");
        let moved = root.path().join("outside.ext");
        fs::rename(&ext, &moved)?;
        std::os::unix::fs::symlink(&moved, &ext)?;
        assert!(inspect(&share, LOCATION).is_err());
        Ok(())
    }
}
