//! A classic USB DAT/EXT pair can carry the finite beats, including microseconds.
//! Unknown revisions retain the existing DAT tempo-segment import and raw companions.

use super::{parse_ppth, pqt2, pqt2_fractions, pqtz_entries};
use crate::model::{BeatGrid, GridAnchor};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

// At 120 BPM this covers more than nine hours. Longer grids retain DAT import.
const MAX_RECORDS: usize = u16::MAX as usize;

/// Read only the media path and one grid section. Waveforms can dwarf the grid.
fn member(mut file: impl Read + Seek, tag: &[u8; 4], limit: usize) -> Option<(String, Vec<u8>)> {
    let len = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0; 12];
    file.read_exact(&mut header).ok()?;
    let number = |v: &[u8]| u64::from(u32::from_be_bytes(v.try_into().unwrap()));
    if &header[..4] != b"PMAI" || number(&header[4..8]) != 28 || number(&header[8..]) != len {
        return None;
    }
    let (mut media, mut grid) = (None, None);
    let mut offset = 28;
    for _ in 0..1024 {
        if offset == len { break; }
        if offset + 12 > len { return None; }
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut header).ok()?;
        let size = number(&header[8..]);
        if !(12..=size).contains(&number(&header[4..8])) || size > len - offset { return None; }
        let path = &header[..4] == b"PPTH";
        if path || header.starts_with(b"PQT") {
            if path && (media.is_some() || size > 65536) ||
                !path && (&header[..4] != tag || grid.is_some() || size > limit as u64) {
                return None;
            }
            let mut bytes = vec![0; size as usize];
            bytes[..12].copy_from_slice(&header);
            file.read_exact(&mut bytes[12..]).ok()?;
            if path {
                let recorded = parse_ppth(&bytes)?;
                if recorded.is_empty() { return None; }
                media = Some(recorded);
            } else {
                grid = Some(bytes);
            }
        }
        offset += size;
    }
    if offset != len { return None; }
    Some((media?, grid?))
}

/// Recognize only the paired fractional layout that the classic USB writer emits.
/// The desktop revision's 16-bit row values have a different interpretation.
pub(crate) fn read(dat: impl Read + Seek, ext: impl Read + Seek) -> Option<BeatGrid> {
    let (media, dat) = member(dat, b"PQTZ", 24 + MAX_RECORDS * 8)?;
    let (mirror_media, ext) = member(ext, b"PQT2", 56 + MAX_RECORDS * 2)?;
    if super::super::usb::path_key(&media) != super::super::usb::path_key(&mirror_media) ||
        dat.get(12..20)? != [0, 0, 0, 0, 0, 8, 0, 0] || ext.get(16..20)? != 0x0100_0002u32.to_be_bytes() {
        return None;
    }
    // Validate the count and ordering before allocating either record list.
    if !crate::rbimport::valid_pqtz_section(&dat) { return None; }
    let entries = pqtz_entries(&dat);
    if ext.len() != 56 + entries.len() * 2 { return None; }
    let fractions = pqt2_fractions(&ext);
    // Includes first/last record, count, checksum, reserved bytes and fraction range.
    if pqt2(&entries, &fractions) != ext { return None; }
    let beats = entries.into_iter().zip(fractions).map(|((bar, bpm, ms), fraction)| GridAnchor {
        time: (u64::from(ms) * 1000 + u64::from(fraction)) as f64 / 1e6,
        bpm: f64::from(bpm) / 100.0,
        beat: bar as u8,
    }).collect();
    Some(BeatGrid::from_listed(beats))
}

/// A cached native pair can retain original sections only as long as its actual
/// beat list agrees with the local one. Legacy segment grids keep their DAT check.
pub(crate) fn matches_source(dat: &Path, grid: &BeatGrid) -> bool {
    if !grid.has_listed_beats() {
        return crate::rbimport::read_pqtz_grid(dat).is_some_and(|g| crate::app::grids_match(&g, grid));
    }
    let regular = |p: &Path| {
        if !p.metadata().ok()?.is_file() { return None; }
        std::fs::File::open(p).ok()
    };
    regular(dat).zip(regular(&dat.with_extension("EXT")))
        .and_then(|(dat, ext)| read(dat, ext)).is_some_and(|g| crate::app::grids_match(&g, grid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Result as IoResult};

    #[test]
    fn only_reads_the_path_and_grid_and_rejects_ambiguous_members() {
        struct CountingReader<'a>(Cursor<&'a [u8]>, usize);
        impl Read for CountingReader<'_> {
            fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
                let n = self.0.read(buf)?;
                self.1 += n;
                Ok(n)
            }
        }
        impl Seek for CountingReader<'_> {
            fn seek(&mut self, pos: SeekFrom) -> IoResult<u64> { self.0.seek(pos) }
        }
        let path = super::super::ppth("/Contents/test.wav");
        let raw = super::super::pqtz(&[crate::model::Beat { index: 0, time: 0.001234, bpm: 123.45, beat: 4 }]);
        let wave = super::super::section(b"PWV5", &[], &vec![0; 2 * 1024 * 1024]);
        let dat = super::super::file(vec![path.clone(), wave, raw.clone()]);
        let mut input = CountingReader(Cursor::new(&dat), 0);
        assert_eq!(member(&mut input, b"PQTZ", 128).unwrap().1, raw);
        assert!(input.1 < 256, "read {} bytes including section headers", input.1);
        assert!(member(Cursor::new(&dat), b"PQTZ", raw.len() - 1).is_none());
        let dup = super::super::file(vec![path, raw.clone(), raw]);
        assert!(member(Cursor::new(dup), b"PQTZ", 128).is_none());
    }
}
