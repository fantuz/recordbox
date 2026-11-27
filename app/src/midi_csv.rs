//! rekordbox's controller mapping files, `<MIDI port name>.midi.csv`: the line grammar, the
//! option flags and the way rekordbox writes a user's changes back (rekordbox 7.2.19,
//! `MidiMap::analyzeCsvLine` 0x101ecd3f8, `loadCsvLine` 0x101ecd5cc, `createCsv` 0x101ed3b44,
//! `createCsvLine` 0x101ed30f4, `getOptionText` 0x101ed2c58; `checks/re-7.2.19/controllers-midi.md`
//! sections 3 and 5.7).
//!
//! A file is read without losing anything (comments, empty lines, fields past the fifteenth,
//! line ends and a byte order mark are kept), so a file that was not changed is written back
//! byte for byte. A user's mapping is written the way rekordbox writes it: `@file,1,<name>`,
//! then one line per function with full four-digit codes per deck, options in rekordbox's
//! order, CRLF, no column header.
//!
//! The factory files themselves are AlphaTheta's and are never part of recordbox: they are
//! read from the installed rekordbox at run time (`factory_dir`).

use std::collections::BTreeMap;

/// The `Type` column. Unknown or empty text is a Button, as in rekordbox.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Button,
    Toggle,
    Slider,
    Knob,
    KnobSlider,
    KnobSliderHiRes,
    Value,
    RawValue,
    Difference,
    Rotary,
    RotaryAsKnob,
    JogRotate,
    JogRotateAbsPos,
    JogTimestamp,
    JogTouch,
    Indicator,
    IndicatorHiRes,
    JogIndicator,
    JogIndicatorHiRes,
    Parameter,
    Pad,
}

const KINDS: [(Kind, &str, u8); 21] = [
    (Kind::Button, "Button", 0),
    (Kind::Toggle, "Toggle", 27),
    (Kind::Slider, "Slider", 4),
    (Kind::Knob, "Knob", 5),
    (Kind::KnobSlider, "KnobSlider", 6),
    (Kind::KnobSliderHiRes, "KnobSliderHiRes", 7),
    (Kind::Value, "Value", 15),
    (Kind::RawValue, "RawValue", 16),
    (Kind::Difference, "Difference", 12),
    (Kind::Rotary, "Rotary", 13),
    (Kind::RotaryAsKnob, "RotaryAsKnob", 14),
    (Kind::JogRotate, "JogRotate", 23),
    (Kind::JogRotateAbsPos, "JogRotateAbsPos", 24),
    (Kind::JogTimestamp, "JogTimestamp", 25),
    (Kind::JogTouch, "JogTouch", 26),
    (Kind::Indicator, "Indicator", 17),
    (Kind::IndicatorHiRes, "IndicatorHiRes", 18),
    (Kind::JogIndicator, "JogIndicator", 19),
    (Kind::JogIndicatorHiRes, "JogIndicatorHiRes", 20),
    (Kind::Parameter, "Parameter", 28),
    (Kind::Pad, "Pad", 33),
];

impl Kind {
    pub fn from_text(s: &str) -> Kind {
        KINDS.iter().find(|k| k.1 == s.trim()).map(|k| k.0).unwrap_or(Kind::Button)
    }
    pub fn text(self) -> &'static str {
        KINDS.iter().find(|k| k.0 == self).map(|k| k.1).unwrap_or("Button")
    }
    /// rekordbox's `eSwitchType` number (a Pad is 29 to 33 by device family; 33 here).
    pub fn switch_type(self) -> u8 {
        KINDS.iter().find(|k| k.0 == self).map(|k| k.2).unwrap_or(0)
    }
    /// The control sends a 14-bit pair: MSB on the code, LSB on the code + 0x20.
    pub fn is_14bit(self) -> bool {
        matches!(self, Kind::KnobSliderHiRes | Kind::Slider | Kind::Knob)
    }
    /// Output only: the row has no input side.
    pub fn output_only(self) -> bool {
        matches!(self, Kind::Indicator | Kind::IndicatorHiRes | Kind::JogIndicator | Kind::JogIndicatorHiRes)
    }
    /// The row can light something (rekordbox's mask 0x3E0460001 over the switch type).
    pub fn can_light(self) -> bool {
        matches!(self, Kind::Button | Kind::Indicator | Kind::IndicatorHiRes | Kind::Pad)
    }
}

/// The `Option` column. Defaults: Blink 0, Value / Min / Max -1, Priority 0.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Opts {
    pub ro: bool,
    pub fast: bool,
    pub dual: bool,
    pub blink: i32,
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub priority: i32,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { ro: false, fast: false, dual: false, blink: 0, value: -1, min: -1, max: -1, priority: 0 }
    }
}

impl Opts {
    /// `;`-separated, no quoting, names in any case; a name without `=value` counts as 1;
    /// unknown names are skipped.
    pub fn parse(text: &str) -> Opts {
        let mut o = Opts::default();
        for part in text.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (k, v) = match part.split_once('=') {
                Some((k, v)) if !v.trim().is_empty() => (k.trim(), v.trim()),
                Some((k, _)) => (k.trim(), "1"),
                None => (part, "1"),
            };
            let n = v.parse::<i32>().unwrap_or(0);
            match k.to_ascii_lowercase().as_str() {
                "ro" => o.ro = n != 0,
                "fast" => o.fast = n != 0,
                "dual" => o.dual = n != 0,
                "blink" => o.blink = n,
                "value" => o.value = n,
                "min" => o.min = n,
                "max" => o.max = n,
                "priority" => o.priority = n,
                _ => {}
            }
        }
        o
    }

    /// rekordbox's own text: `RO;Fast;Blink=;Value=;Min=;Max=;Priority=;Dual;`, each only when
    /// it is not the default, each followed by its semicolon.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ro {
            s.push_str("RO;");
        }
        if self.fast {
            s.push_str("Fast;");
        }
        if self.blink != 0 {
            s.push_str(&format!("Blink={};", self.blink));
        }
        if self.value != -1 {
            s.push_str(&format!("Value={};", self.value));
        }
        if self.min != -1 {
            s.push_str(&format!("Min={};", self.min));
        }
        if self.max != -1 {
            s.push_str(&format!("Max={};", self.max));
        }
        if self.priority != 0 {
            s.push_str(&format!("Priority={};", self.priority));
        }
        if self.dual {
            s.push_str("Dual;");
        }
        s
    }
}

/// One mapping line. The fifteen cells are kept as written; `input` / `output` are the codes
/// they stand for (index 0 = no deck, 1..4 = the decks).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub cells: Vec<String>,
    pub name: String,
    pub function: String,
    pub kind: Kind,
    pub input: [Option<u16>; 5],
    pub output: [Option<u16>; 5],
    pub opts: Opts,
    pub comment: String,
}

fn hex(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    u32::from_str_radix(s, 16).ok().map(|v| v as u16)
}

/// The codes of a base cell and its four deck cells (`loadCsvLine`):
/// base and deck cell: the deck cell is a decimal number added to the status byte;
/// no base: each deck cell is a full code; base alone: one mapping without a deck.
/// A JogTimestamp row has a two-digit base and the deck cell goes into its low bits.
pub fn codes(base: &str, decks: [&str; 4], kind: Kind) -> [Option<u16>; 5] {
    let mut out = [None; 5];
    let any_deck = decks.iter().any(|d| !d.trim().is_empty());
    match hex(base) {
        Some(b) => {
            if !any_deck {
                out[0] = Some(b);
            }
            for (i, d) in decks.iter().enumerate() {
                if let Ok(n) = d.trim().parse::<u32>() {
                    out[i + 1] = Some(if kind == Kind::JogTimestamp { b | n as u16 } else { b | ((n as u16) << 8) });
                }
            }
        }
        None => {
            for (i, d) in decks.iter().enumerate() {
                out[i + 1] = hex(d);
            }
        }
    }
    out
}

impl Row {
    /// A line of at least fifteen comma-separated fields with a name; no quoting.
    pub fn parse(line: &str) -> Option<Row> {
        let cells: Vec<String> = line.split(',').map(|s| s.to_string()).collect();
        if cells.len() < 15 || cells[0].is_empty() || cells[0].starts_with('#') || cells[0].starts_with('@') {
            return None;
        }
        let kind = Kind::from_text(&cells[2]);
        let c = |i: usize| cells[i].as_str();
        let input = codes(c(3), [c(4), c(5), c(6), c(7)], kind);
        let output = codes(c(8), [c(9), c(10), c(11), c(12)], kind);
        Some(Row { name: cells[0].trim().to_string(), function: cells[1].clone(), kind, input, output, opts: Opts::parse(c(13)), comment: cells[14].clone(), cells })
    }

    /// A new row in rekordbox's own layout (full codes per deck).
    pub fn new(name: &str, function: &str, kind: Kind, input: [Option<u16>; 5], output: [Option<u16>; 5], opts: Opts, comment: &str) -> Row {
        let mut cells = vec![String::new(); 15];
        cells[0] = name.to_string();
        cells[1] = function.to_string();
        cells[2] = kind.text().to_string();
        for i in 0..5 {
            cells[3 + i] = input[i].map(|c| format!("{c:04X}")).unwrap_or_default();
            cells[8 + i] = output[i].map(|c| format!("{c:04X}")).unwrap_or_default();
        }
        cells[13] = opts.text();
        cells[14] = comment.to_string();
        Row { cells, name: name.to_string(), function: function.to_string(), kind, input, output, opts, comment: comment.to_string() }
    }

    pub fn line(&self) -> String {
        self.cells.join(",")
    }
}

/// One line of a file as read.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub raw: String,
    /// "\r\n", "\n" or "" (a last line without one).
    pub end: String,
    pub row: Option<Row>,
}

#[derive(Debug, PartialEq)]
pub enum CsvError {
    /// No `@file` line, or its version is not 1.
    NotSupported,
}

/// A mapping file as read: every line, the device name of its `@file` line.
#[derive(Clone, Debug, PartialEq)]
pub struct CsvFile {
    pub bom: bool,
    pub device: String,
    pub lines: Vec<Line>,
}

impl CsvFile {
    pub fn parse(bytes: &[u8]) -> Result<CsvFile, CsvError> {
        let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
            Some(b) => (true, b),
            None => (false, bytes),
        };
        let text = String::from_utf8_lossy(body);
        let mut lines = vec![];
        let mut device = None;
        let mut rest: &str = &text;
        while !rest.is_empty() {
            let (line, end, next) = match rest.find('\n') {
                Some(i) => {
                    let l = &rest[..i];
                    match l.strip_suffix('\r') {
                        Some(l2) => (l2, "\r\n", &rest[i + 1..]),
                        None => (l, "\n", &rest[i + 1..]),
                    }
                }
                None => (rest, "", ""),
            };
            rest = next;
            let mut row = None;
            if line.starts_with('#') {
                // A comment (a commented-out row too).
            } else if line.len() >= 5 && line[..5].eq_ignore_ascii_case("@file") {
                let f: Vec<&str> = line.split(',').collect();
                if f.get(1).map(|v| v.trim()) != Some("1") {
                    return Err(CsvError::NotSupported);
                }
                if device.is_none() {
                    device = Some(f.get(2).map(|s| s.trim().to_string()).unwrap_or_default());
                }
            } else {
                row = Row::parse(line);
            }
            lines.push(Line { raw: line.to_string(), end: end.to_string(), row });
        }
        Ok(CsvFile { bom, device: device.ok_or(CsvError::NotSupported)?, lines })
    }

    /// The file as it was read (changed rows as they are now).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = if self.bom { vec![0xEF, 0xBB, 0xBF] } else { vec![] };
        for l in &self.lines {
            match &l.row {
                Some(r) => out.extend_from_slice(r.line().as_bytes()),
                None => out.extend_from_slice(l.raw.as_bytes()),
            }
            out.extend_from_slice(l.end.as_bytes());
        }
        out
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.lines.iter().filter_map(|l| l.row.as_ref())
    }
}

/// One code of one deck with what it does: what rekordbox keeps per table slot
/// (`MidiDataInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub function: String,
    pub kind: Kind,
    /// 0 = no deck, 1..4.
    pub deck: u8,
    pub code: u16,
    pub opts: Opts,
    pub comment: String,
    /// Changed by the user (only these are written to the user's file).
    pub user: bool,
}

/// The entries of a row: its inputs and its outputs.
pub fn entries(row: &Row, user: bool) -> (Vec<Entry>, Vec<Entry>) {
    let mk = |deck: usize, code: u16| Entry { name: row.name.clone(), function: row.function.clone(), kind: row.kind, deck: deck as u8, code, opts: row.opts, comment: row.comment.clone(), user };
    let ins = (0..5).filter_map(|d| row.input[d].map(|c| mk(d, c))).collect();
    let outs = (0..5).filter_map(|d| row.output[d].map(|c| mk(d, c))).collect();
    (ins, outs)
}

fn key(e: &Entry) -> String {
    let o = &e.opts;
    format!("{}:{}:{}:{}:{}:{}:{}:{}:{}:{}", e.name, e.kind.switch_type(), if o.ro { "ro" } else { "normal" }, if o.fast { "fast" } else { "normal" }, if o.dual { "dual" } else { "normal" }, o.blink, o.value, o.min, o.max, o.priority)
}

type Holder = [Option<Entry>; 5];

/// rekordbox's `pushInfo`: entries with the same name, type and options share a line, one
/// code per deck; a second code for the same deck starts another line.
fn hold(list: &[Entry]) -> BTreeMap<String, Vec<Holder>> {
    let mut map: BTreeMap<String, Vec<Holder>> = BTreeMap::new();
    for e in list {
        let v = map.entry(key(e)).or_default();
        let d = (e.deck as usize).min(4);
        match v.iter_mut().find(|h| h[d].is_none()) {
            Some(h) => h[d] = Some(e.clone()),
            None => {
                let mut h: Holder = Default::default();
                h[d] = Some(e.clone());
                v.push(h);
            }
        }
    }
    map
}

fn first(h: &Holder) -> Option<&Entry> {
    h.iter().flatten().next()
}

/// The user's mapping as rekordbox saves it (`MidiMap::save` 0x101ed3e78): the header, then
/// one line per function sorted by name. An input line carries its outputs when a line of
/// outputs has the same name, type and options and the same code on its first deck.
pub fn save_delta(device: &str, inputs: &[Entry], outputs: &[Entry]) -> String {
    let mut ins: Vec<Entry> = inputs.iter().filter(|e| e.user).cloned().collect();
    ins.sort_by_key(|e| e.code);
    let mut outs: Vec<Entry> = outputs.iter().filter(|e| e.user).cloned().collect();
    outs.sort_by(|a, b| (a.deck, &a.name, a.code).cmp(&(b.deck, &b.name, b.code)));
    let hi = hold(&ins);
    let mut ho = hold(&outs);
    // (name, input line, output line), kept in name order.
    let mut lines: Vec<(String, Option<Holder>, Option<Holder>)> = vec![];
    for (k, list) in hi {
        for h in list {
            let Some(f) = first(&h) else { continue };
            let d = f.deck as usize;
            let mut bound = None;
            if let Some(cands) = ho.get_mut(&k) {
                if let Some(i) = cands.iter().position(|o| o[d].as_ref().map(|e| e.code) == Some(f.code)) {
                    bound = Some(cands.remove(i));
                }
            }
            lines.push((f.name.clone(), Some(h), bound));
        }
    }
    for (_, list) in ho {
        for h in list {
            if let Some(f) = first(&h) {
                lines.push((f.name.clone(), None, Some(h)));
            }
        }
    }
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = format!("@file,1,{device}\r\n");
    for (_, i, o) in lines {
        let lead = i.as_ref().and_then(first).or_else(|| o.as_ref().and_then(first));
        let Some(lead) = lead else { continue };
        let cell = |h: &Option<Holder>, d: usize| h.as_ref().and_then(|h| h[d].as_ref()).map(|e| format!("{:04X}", e.code)).unwrap_or_default();
        let mut cells = vec![lead.name.clone(), lead.function.clone(), lead.kind.text().to_string()];
        for d in 0..5 {
            cells.push(cell(&i, d));
        }
        for d in 0..5 {
            cells.push(cell(&o, d));
        }
        cells.push(lead.opts.text());
        cells.push(lead.comment.clone());
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// A user file as rekordbox makes it the first time a device is seen: the header only.
pub fn empty_user_file(device: &str) -> String {
    format!("@file,1,{device}\r\n")
}

/// rekordbox's export file name: the device name with `\ / : * ? " < > |` replaced by `_`,
/// and `.csv`.
fn device_filename(device: &str) -> String {
    device.chars().map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect()
}

pub fn export_name(device: &str) -> String {
    format!("{}.csv", device_filename(device))
}

pub fn mapping_name(device: &str) -> String {
    format!("{}.midi.csv", device_filename(device))
}

/// Where the installed rekordbox keeps its factory mappings (read only, never copied).
pub fn factory_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("PREPDECK_MIDI_FACTORY") {
        return d.into();
    }
    "/Applications/rekordbox 7/rekordbox.app/Contents/Resources/MidiMappings".into()
}

/// The factory mapping of a port name: `<name>.midi.csv`, and for a second identical unit
/// (a name ending in a space and digits) the file of the name without that ending.
pub fn factory_file(dir: &std::path::Path, port: &str) -> Option<std::path::PathBuf> {
    let p = dir.join(mapping_name(port));
    if p.is_file() {
        return Some(p);
    }
    let t = port.trim_end_matches(|c: char| c.is_ascii_digit());
    if t.len() < port.len() && t.ends_with(' ') {
        let p = dir.join(mapping_name(t.trim_end()));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_cells_are_offsets_full_codes_or_absent() {
        // Base and deck offsets: the offset goes into the status byte.
        let r = Row::parse("PlayPause,PlayPause,Button,900B,0,1,2,3,900B,0,1,,,Fast;Priority=50,Play/Pause").unwrap();
        assert_eq!(r.input, [None, Some(0x900B), Some(0x910B), Some(0x920B), Some(0x930B)]);
        assert_eq!(r.output, [None, Some(0x900B), Some(0x910B), None, None]);
        assert_eq!(r.kind, Kind::Button);
        assert!(r.opts.fast && r.opts.priority == 50 && !r.opts.ro);
        // Pads: channel offsets 7, 9, 11, 13.
        let r = Row::parse("PAD1_HotCue,PAD1,Pad,9000,7,9,11,13,9000,7,9,11,13,Fast,").unwrap();
        assert_eq!(r.input, [None, Some(0x9700), Some(0x9900), Some(0x9B00), Some(0x9D00)]);
        // No base: full codes per deck.
        let r = Row::parse("Load,Load,Button,,9646,9647,9648,9649,,,,,,,").unwrap();
        assert_eq!(r.input, [None, Some(0x9646), Some(0x9647), Some(0x9648), Some(0x9649)]);
        // Base alone: one mapping without a deck.
        let r = Row::parse("CrossFader,CrossFader,KnobSliderHiRes,B61F,,,,,,,,,,,").unwrap();
        assert_eq!(r.input, [Some(0xB61F), None, None, None, None]);
        assert!(r.kind.is_14bit());
        // JogTimestamp: a two-digit base, the deck cell in its low bits.
        let r = Row::parse("JogTimestamp,JogTimestamp,JogTimestamp,E0,0,1,,,,,,,,RO,").unwrap();
        assert_eq!(r.input, [None, Some(0xE0), Some(0xE1), None, None]);
        // Unknown type text is a Button; fewer than 15 fields, an empty name or a '#' is no row.
        assert_eq!(Row::parse("X,Y,Whatever,9001,,,,,,,,,,,").unwrap().kind, Kind::Button);
        assert!(Row::parse("X,Y,Button,9001").is_none());
        assert!(Row::parse(",,,,,,,,,,,,,,").is_none());
        assert!(Row::parse("#WheelSearch,WheelSearch,JogRotate,B026,0,1,,,,,,,,RO,Pitch Bend").is_none());
        // More than 15 fields: the first 15 count, the rest is kept.
        let r = Row::parse("A,B,Button,9001,,,,,,,,,,,c,x,y,z,w").unwrap();
        assert_eq!(r.comment, "c");
        assert_eq!(r.line(), "A,B,Button,9001,,,,,,,,,,,c,x,y,z,w");
    }

    #[test]
    fn options_parse_and_are_written_in_rekordboxs_order() {
        let o = Opts::parse("Dual;Priority=100;Max=16383;Min=1;Value=130;Blink=500;fast;RO");
        assert_eq!(o, Opts { ro: true, fast: true, dual: true, blink: 500, value: 130, min: 1, max: 16383, priority: 100 });
        assert_eq!(o.text(), "RO;Fast;Blink=500;Value=130;Min=1;Max=16383;Priority=100;Dual;");
        assert_eq!(Opts::parse(""), Opts::default());
        assert_eq!(Opts::default().text(), "");
        // Unknown names are skipped; a name with an empty value counts as 1.
        assert_eq!(Opts::parse("Colour=3;Fast=;").text(), "Fast;");
        assert_eq!(Opts::parse(&o.text()), o);
    }

    #[test]
    fn a_file_reads_and_writes_back_unchanged() {
        let src = b"\xEF\xBB\xBF@file,1,My Device,,,,,,,,,,,,\r\n#name,function,type\r\n,,,,,,,,,,,,,,\r\n# Deck,,,,,,,,,,,,,,\r\nPlayPause,PlayPause,Button,900B,0,1,,,900B,0,1,,,Fast;Priority=50,Play/Pause\r\nlast,line,Button,9001,,,,,,,,,,,";
        let f = CsvFile::parse(src).unwrap();
        assert!(f.bom);
        assert_eq!(f.device, "My Device");
        assert_eq!(f.rows().count(), 2);
        assert_eq!(f.to_bytes(), src.to_vec());
        assert_eq!(CsvFile::parse(b"PlayPause,PlayPause,Button,900B,,,,,,,,,,,\r\n"), Err(CsvError::NotSupported));
        assert_eq!(CsvFile::parse(b"@file,2,X\r\n"), Err(CsvError::NotSupported));
    }

    #[test]
    fn the_users_file_is_written_as_rekordbox_writes_it() {
        let e = |name: &str, kind: Kind, deck: u8, code: u16, opts: Opts| Entry { name: name.into(), function: name.into(), kind, deck, code, opts, comment: String::new(), user: true };
        let fast = Opts { fast: true, blink: 1000, priority: 50, ..Default::default() };
        let ins = vec![e("PlayPause", Kind::Button, 1, 0x900B, fast), e("PlayPause", Kind::Button, 2, 0x910B, fast), e("CrossFader", Kind::KnobSlider, 0, 0xB61F, Opts { fast: true, ..Default::default() }), e("NoFunction", Kind::Button, 1, 0x9010, Opts::default())];
        let mut outs = vec![e("PlayPause", Kind::Button, 1, 0x900B, fast), e("PlayPause", Kind::Button, 2, 0x910B, fast), e("TempoUp", Kind::Indicator, 1, 0x9034, Opts::default())];
        // A factory entry (not changed by the user) is not written.
        outs.push(Entry { user: false, ..e("Sync", Kind::Button, 1, 0x9058, Opts::default()) });
        let text = save_delta("My Device", &ins, &outs);
        assert_eq!(
            text,
            "@file,1,My Device\r\n\
             CrossFader,CrossFader,KnobSlider,B61F,,,,,,,,,,Fast;,\r\n\
             NoFunction,NoFunction,Button,,9010,,,,,,,,,,\r\n\
             PlayPause,PlayPause,Button,,900B,910B,,,,900B,910B,,,Fast;Blink=1000;Priority=50;,\r\n\
             TempoUp,TempoUp,Indicator,,,,,,,9034,,,,,\r\n"
        );
        // It reads back to the same entries.
        let f = CsvFile::parse(text.as_bytes()).unwrap();
        let (mut i2, mut o2) = (vec![], vec![]);
        for r in f.rows() {
            let (a, b) = entries(r, true);
            i2.extend(a);
            o2.extend(b);
        }
        let norm = |v: &[Entry]| {
            let mut k: Vec<(String, u8, u16, Kind, Opts)> = v.iter().filter(|e| e.user).map(|e| (e.name.clone(), e.deck, e.code, e.kind, e.opts)).collect();
            k.sort_by(|a, b| (&a.0, a.1, a.2).cmp(&(&b.0, b.1, b.2)));
            k
        };
        assert_eq!(norm(&i2), norm(&ins));
        assert_eq!(norm(&o2), norm(&outs));
        // Two codes of one function on the same deck take two lines.
        let two = vec![e("PlayPause", Kind::Button, 1, 0x900B, fast), e("PlayPause", Kind::Button, 1, 0x900E, fast)];
        assert_eq!(save_delta("D", &two, &[]).lines().count(), 3);
        assert_eq!(export_name("XDJ-XZ (1)/a:b"), "XDJ-XZ (1)_a_b.csv");
        assert_eq!(mapping_name("Studio/A:B"), "Studio_A_B.midi.csv");
        assert_eq!(empty_user_file("X"), "@file,1,X\r\n");
    }

    /// Every factory file of the installed rekordbox: read, write back byte for byte; every
    /// row's options survive rekordbox's own option text; and the whole mapping written the
    /// way rekordbox saves a user file reads back to the same codes. Skipped when rekordbox
    /// is not installed.
    #[test]
    fn factory_files_round_trip() {
        let dir = factory_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            eprintln!("factory_files_round_trip: skipped, {} not found", dir.display());
            return;
        };
        let (mut files, mut rows, mut canonical, mut entries_n, mut bare_lf) = (0usize, 0usize, 0, 0, 0);
        // Mapping files rekordbox added after 7.2.19 (the version the counts were taken from).
        const NEWER_FILES: [&str; 1] = ["DJM-A7"];
        let mut newer: Vec<(String, usize)> = vec![];
        let mut kinds: std::collections::HashMap<&'static str, usize> = Default::default();
        for ent in rd.flatten() {
            let p = ent.path();
            let Some(fname) = p.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else { continue };
            let Some(port) = fname.strip_suffix(".midi.csv") else { continue };
            let bytes = std::fs::read(&p).unwrap();
            let f = CsvFile::parse(&bytes).unwrap_or_else(|e| panic!("{fname}: {e:?}"));
            files += 1;
            if NEWER_FILES.contains(&port) {
                newer.push((port.to_string(), f.rows().count()));
            }
            assert_eq!(f.to_bytes(), bytes, "{fname} does not write back unchanged");
            assert!(f.device.eq_ignore_ascii_case(port), "{fname}: @file names {}", f.device);
            bare_lf += f.lines.iter().filter(|l| l.end == "\n").count();
            let (mut ins, mut outs) = (vec![], vec![]);
            for r in f.rows() {
                rows += 1;
                *kinds.entry(r.kind.text()).or_default() += 1;
                // The type text is one of the vocabulary (or one of the four rows without one).
                assert!(r.cells[2].is_empty() || Kind::from_text(&r.cells[2]).text() == r.cells[2] || r.cells[2].starts_with(port.split(' ').next().unwrap_or("")) || r.kind == Kind::Button, "{fname}: type {}", r.cells[2]);
                assert_eq!(Opts::parse(&r.opts.text()), r.opts, "{fname}: {}", r.line());
                if r.opts.text().trim_end_matches(';') == r.cells[13].trim_end_matches(';') {
                    canonical += 1;
                }
                // The same row in rekordbox's own layout (full codes) means the same codes.
                let again = Row::parse(&Row::new(&r.name, &r.function, r.kind, r.input, r.output, r.opts, &r.comment).line());
                if r.kind != Kind::JogTimestamp && !r.comment.contains(',') {
                    let again = again.unwrap_or_else(|| panic!("{fname}: {}", r.line()));
                    assert_eq!((again.input, again.output, again.opts, again.kind), (r.input, r.output, r.opts, r.kind), "{fname}: {}", r.line());
                }
                let (a, b) = entries(r, true);
                ins.extend(a);
                outs.extend(b);
            }
            entries_n += ins.len() + outs.len();
            // Saved as a user file and read again: the same set of (name, deck, code, type, options).
            let saved = save_delta(&f.device, &ins, &outs);
            assert!(saved.starts_with(&format!("@file,1,{}\r\n", f.device)));
            assert!(!saved.replace("\r\n", "").contains('\n'), "{fname}: bare LF in the saved file");
            let g = CsvFile::parse(saved.as_bytes()).unwrap();
            let (mut i2, mut o2) = (vec![], vec![]);
            for r in g.rows() {
                let (a, b) = entries(r, true);
                i2.extend(a);
                o2.extend(b);
            }
            let norm = |v: &[Entry]| {
                let mut k: Vec<(String, u8, u16, u8, Opts)> = v.iter().filter(|e| e.kind != Kind::JogTimestamp).map(|e| (e.name.clone(), e.deck, e.code, e.kind.switch_type(), e.opts)).collect();
                k.sort_by(|a, b| (&a.0, a.1, a.2, a.3).cmp(&(&b.0, b.1, b.2, b.3)));
                k.dedup();
                k
            };
            assert_eq!(norm(&i2), norm(&ins), "{fname}: inputs after a save");
            assert_eq!(norm(&o2), norm(&outs), "{fname}: outputs after a save");
        }
        eprintln!("factory_files_round_trip: {files} files, {rows} rows ({canonical} with options already in rekordbox's order), {entries_n} codes, {bare_lf} lines ending in a bare LF; kinds {kinds:?}");
        // rekordbox 7.2.19 shipped 69 files with 13,505 rows; 7.2.20 (installed on this machine
        // on 2026-11-27) adds DJM-A7.midi.csv (12 rows). Every file found was checked above;
        // the 7.2.19 set must still be there unchanged in size.
        assert!(files >= 69, "{files} mapping files: fewer than rekordbox 7.2.19 shipped");
        assert_eq!(files - newer.len(), 69, "files added after 7.2.19 other than {NEWER_FILES:?}");
        assert_eq!(rows - newer.iter().map(|x| x.1).sum::<usize>(), 13505, "rows of the 7.2.19 files");
    }

    #[test]
    fn a_second_identical_unit_uses_the_first_ones_file() {
        let d = std::env::temp_dir().join(format!("recordbox-midi-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("DDJ-FLX4.midi.csv"), "@file,1,DDJ-FLX4\r\n").unwrap();
        assert!(factory_file(&d, "DDJ-FLX4").is_some());
        assert!(factory_file(&d, "DDJ-FLX4 2").is_some());
        assert!(factory_file(&d, "DDJ-FLX42").is_none());
        assert!(factory_file(&d, "Launchpad").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
