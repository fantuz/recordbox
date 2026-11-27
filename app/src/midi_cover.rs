//! Read-only coverage of the inputs and outputs declared in one controller CSV.

use crate::midi::{input_acts, output_acts};
use crate::midi_csv::{self, Kind};
use crate::midi_map::NO_FUNCTION;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;

pub fn cli() -> bool {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match requested_file(&args) {
        Ok(Some(file)) => {
            if let Err(error) = report(file, io::BufWriter::new(io::stdout().lock())) {
                eprintln!("MIDI coverage failed for {file:?}: {error}");
                std::process::exit(1);
            }
            true
        }
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
        Ok(None) => false,
    }
}

fn requested_file(args: &[OsString]) -> Result<Option<&Path>, &'static str> {
    const FLAG: &str = "--midi-coverage";
    if !args.iter().any(|arg| arg == FLAG) {
        return Ok(None);
    }
    match args {
        [flag, file]
            if flag == FLAG && !file.is_empty() && !file.to_string_lossy().starts_with("--") =>
        {
            Ok(Some(Path::new(file)))
        }
        _ => Err("Usage: recordbox --midi-coverage FILE"),
    }
}

#[derive(Default)]
struct Counts {
    supported: usize,
    unsupported: usize,
    disabled: usize,
}

impl Counts {
    fn classify(&mut self, name: &str, supported: bool) -> &'static str {
        if name == NO_FUNCTION {
            self.disabled += 1;
            "disabled"
        } else if supported {
            self.supported += 1;
            "supported"
        } else {
            self.unsupported += 1;
            "unsupported"
        }
    }

    fn write(&self, out: &mut impl Write, label: &str) -> io::Result<()> {
        writeln!(
            out,
            "{label}: {} supported, {} unsupported, {} disabled",
            self.supported, self.unsupported, self.disabled
        )
    }
}

// Names come from the file and can contain terminal control characters.
fn display_field(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

pub fn report(file: &Path, mut out: impl Write) -> io::Result<()> {
    let csv = midi_csv::parse_mapping(&midi_csv::read_file(file)?)?;
    writeln!(out, "MIDI mapping: {}", display_field(&csv.device))?;
    writeln!(
        out,
        "Coverage counts each declared row once per direction, across all deck codes."
    )?;

    let (mut inputs, mut outputs, mut settings) =
        (Counts::default(), Counts::default(), Counts::default());
    let mut rows = 0;
    let mut unassigned = 0;
    for (i, line) in csv.lines.iter().enumerate() {
        let Some(row) = &line.row else { continue };
        rows += 1;
        let declared_type = row.cells[2].trim();
        let kind = row.kind.text();
        let type_label = if declared_type.is_empty() || declared_type == kind {
            kind.to_owned()
        } else {
            format!("{} (read as {kind})", display_field(declared_type))
        };
        write!(
            out,
            "Line {}  {} [{}]",
            i + 1,
            display_field(&row.name),
            type_label
        )?;
        if row.kind == Kind::Parameter {
            let state = settings.classify(&row.name, input_acts(&row.name, row.kind));
            writeln!(out, "  setting {state}")?;
            continue;
        }
        let has_input = row.input.iter().any(Option::is_some);
        let has_output = row.output.iter().any(Option::is_some);
        if has_input {
            let state = inputs.classify(&row.name, input_acts(&row.name, row.kind));
            write!(out, "  input {state}")?;
        }
        if has_output {
            let state = outputs.classify(&row.name, output_acts(&row.name, row.kind));
            write!(out, "  output {state}")?;
        }
        if !has_input && !has_output {
            unassigned += 1;
            write!(out, "  no assigned codes")?;
        }
        writeln!(out)?;
    }
    writeln!(out, "Mapping rows: {rows}, unassigned: {unassigned}")?;
    inputs.write(&mut out, "Input rows")?;
    outputs.write(&mut out, "Output rows")?;
    settings.write(&mut out, "Device settings")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_csv::{Opts, Row};

    #[test]
    fn requires_one_explicit_file_for_coverage() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(requested_file(&args(&["--stems-status"])), Ok(None));
        assert_eq!(
            requested_file(&args(&["--midi-coverage", "My Deck.csv"])),
            Ok(Some(Path::new("My Deck.csv")))
        );
        for invalid in [
            vec!["--midi-coverage"],
            vec!["--midi-coverage", "--midi-coverage"],
            vec!["--midi-coverage", "one", "two"],
            vec!["--stems-status", "--midi-coverage", "one"],
        ] {
            assert!(requested_file(&args(&invalid)).is_err());
        }
    }

    #[test]
    fn reports_declared_directions_types_and_deactivated_codes() {
        let both_decks = [None, Some(0x9001), Some(0x9101), None, None];
        let one = [Some(0x9002), None, None, None, None];
        let none = [None; 5];
        let row = |name, kind, ins, outs| {
            Row::new(name, name, kind, ins, outs, Opts::default(), "").line()
        };
        let csv = [
            "@file,1,Test deck".into(),
            row("PlayPause", Kind::Button, both_decks, both_decks),
            row("PlayPause", Kind::KnobSlider, one, none),
            row("MissingAction", Kind::Button, one, both_decks),
            row(NO_FUNCTION, Kind::Button, both_decks, none),
            row("ChannelLevel", Kind::Indicator, none, one),
            row("Unassigned", Kind::Button, none, none),
            row("MidiOutInterval", Kind::Parameter, one, none),
            row("JogRatency", Kind::Parameter, one, none),
            row("JogTimestamp", Kind::JogTimestamp, one, none),
        ]
        .join("\r\n");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mapping.csv");
        std::fs::write(&path, &csv).unwrap();
        let mut result = Vec::new();
        report(&path, &mut result).unwrap();
        let result = String::from_utf8(result).unwrap();
        for expected in [
            "Line 2  PlayPause [Button]  input supported  output supported",
            "Line 3  PlayPause [KnobSlider]  input unsupported",
            "Line 6  ChannelLevel [Indicator]  output supported",
            "Mapping rows: 9, unassigned: 1",
            "Input rows: 2 supported, 2 unsupported, 1 disabled",
            "Output rows: 2 supported, 1 unsupported, 0 disabled",
            "Device settings: 1 supported, 1 unsupported",
        ] {
            assert!(result.contains(expected), "missing {expected} in {result}");
        }
        assert_eq!(std::fs::read_to_string(path).unwrap(), csv);
    }

    #[test]
    fn empty_valid_mapping_and_malformed_input_are_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");
        assert!(report(&path, Vec::new()).is_err());
        std::fs::write(&path, "@file,1,Deck\r\n# empty mapping\r\n").unwrap();
        let mut output = Vec::new();
        report(&path, &mut output).unwrap();
        assert!(String::from_utf8(output)
            .unwrap()
            .contains("Mapping rows: 0, unassigned: 0"));
        for invalid in ["", "@file,1,Deck\ninvalid", "@file,1,Deck\n@file,1,Second"] {
            std::fs::write(&path, invalid).unwrap();
            assert!(report(&path, Vec::new()).is_err());
        }
    }
}
