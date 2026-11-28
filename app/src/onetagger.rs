//! Read-only OneTagger lookup for Related Tracks. Only the artist and title cross
//! the process boundary. The tagging, audio recognition and file commands are separate.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const PROVIDER: &str = "MusicBrainz";
const MAX_TEXT_BYTES: usize = 512;
const MAX_REPLY_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchTags {
    pub genres: Vec<String>,
    pub styles: Vec<String>,
    pub label: String,
    pub provider: String,
    pub recording_id: String,
}

#[derive(Serialize)]
struct Request<'a> {
    artist: &'a str,
    title: &'a str,
}

#[derive(Deserialize)]
struct Reply {
    schema: u32,
    provider: String,
    status: String,
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    recording_id: String,
    artist: String,
    title: String,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    styles: Vec<String>,
    #[serde(default)]
    label: String,
}

fn valid_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_TEXT_BYTES && !s.chars().any(char::is_control)
}

fn identity(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn recording_id(s: &str) -> bool {
    s.len() == 36
        && s != "00000000-0000-0000-0000-000000000000"
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn verified_tags(values: Vec<String>) -> Result<Vec<String>> {
    ensure!(values.len() <= 64, "OneTagger returned too many tag values");
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in values {
        let value = value.trim();
        ensure!(valid_text(value), "OneTagger returned an invalid tag value");
        if seen.insert(identity(value)) {
            result.push(value.to_owned());
        }
    }
    Ok(result)
}

fn decode(bytes: &[u8], request: &Request<'_>) -> Result<Option<MatchTags>> {
    ensure!(
        bytes.len() <= MAX_REPLY_BYTES,
        "OneTagger lookup reply exceeds 64 KiB"
    );
    let reply: Reply = serde_json::from_slice(bytes).context("Invalid OneTagger lookup reply")?;
    ensure!(
        reply.schema == 1 && reply.provider == PROVIDER,
        "Unsupported OneTagger lookup reply"
    );
    match reply.status.as_str() {
        "no_match" | "ambiguous" => return Ok(None),
        "matched" => {}
        _ => bail!("Unknown OneTagger lookup status"),
    }
    ensure!(
        reply.candidates.len() == 1,
        "OneTagger marked a non-unique result as matched"
    );
    let hit = reply.candidates.into_iter().next().unwrap();
    ensure!(
        recording_id(&hit.recording_id)
            && valid_text(&hit.artist)
            && valid_text(&hit.title)
            && identity(&hit.artist) == identity(request.artist)
            && identity(&hit.title) == identity(request.title),
        "OneTagger returned a different recording or edition"
    );
    let genres = verified_tags(hit.genres)?;
    let styles = verified_tags(hit.styles)?;
    let label = hit.label.trim();
    ensure!(
        label.is_empty() || valid_text(label),
        "OneTagger returned an invalid label"
    );
    Ok(
        (!genres.is_empty() || !styles.is_empty() || !label.is_empty()).then(|| MatchTags {
            genres,
            styles,
            label: label.to_owned(),
            provider: reply.provider,
            recording_id: hit.recording_id,
        }),
    )
}

fn check_exit(status: ExitStatus) -> Result<()> {
    match status.code() {
        Some(0) => Ok(()),
        Some(2) => bail!("This executable does not support the read-only OneTagger lookup. Select onetagger-cli with the lookup --json command (OneTagger 2.2.3 predates it)."),
        Some(3) => bail!("MusicBrainz asked OneTagger to pause. Try this lookup again later."),
        _ => bail!("OneTagger could not complete the MusicBrainz lookup ({status})."),
    }
}

/// The caller runs this on the enrichment worker. All send/receive waits are
/// bounded independently of the deck, audio analysis and GUI threads.
pub fn lookup(cli: &Path, artist: &str, title: &str, cancel: &AtomicBool) -> Result<Option<MatchTags>> {
    lookup_with_timeout(cli, artist, title, cancel, TIMEOUT)
}

fn lookup_with_timeout(
    cli: &Path,
    artist: &str,
    title: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Option<MatchTags>> {
    ensure!(!cancel.load(Ordering::Relaxed), "Lookup cancelled");
    ensure!(cli.is_absolute(), "Select the OneTagger CLI executable");
    // Windows may implicitly dispatch .cmd/.bat through cmd.exe.
    #[cfg(windows)]
    ensure!(
        cli.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
        "Select the OneTagger CLI .exe file"
    );
    let request = Request {
        artist: artist.trim(),
        title: title.trim(),
    };
    ensure!(
        valid_text(request.artist) && valid_text(request.title),
        "A lookup requires an artist and title of up to 512 UTF-8 bytes each"
    );

    // stdin is already complete before spawning. An older CLI can exit during
    // argument parsing, and a stalled executable can never block a pipe write.
    let dir = tempfile::tempdir()?;
    let mut input = tempfile::tempfile_in(dir.path())?;
    serde_json::to_writer(&mut input, &request)?;
    input.write_all(b"\n")?;
    input.rewind()?;
    let mut cmd = Command::new(cli);
    cmd.args(["lookup", "--json"])
        .current_dir(dir.path())
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW in the desktop app
    }
    let mut child = cmd
        .spawn()
        .context("Could not start the selected OneTagger CLI")?;
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    let outcome = (|| -> Result<Option<MatchTags>> {
        std::thread::Builder::new()
            .name("onetagger-reply".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                let read = stdout.take(MAX_REPLY_BYTES as u64 + 1).read_to_end(&mut bytes);
                let _ = sender.send(read.map(|_| bytes));
            })?;
        let mut bytes = None;
        loop {
            ensure!(!cancel.load(Ordering::Relaxed), "Lookup cancelled");
            ensure!(started.elapsed() < timeout, "OneTagger lookup timed out");
            if bytes.is_none() {
                match receiver.try_recv() {
                    Ok(result) => {
                        let result = result?;
                        ensure!(
                            result.len() <= MAX_REPLY_BYTES,
                            "OneTagger lookup reply exceeds 64 KiB"
                        );
                        bytes = Some(result);
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => bail!("OneTagger lookup reader stopped"),
                }
            }
            if let Some(status) = child.try_wait()? {
                check_exit(status)?;
                if let Some(output) = &bytes {
                    return decode(output, &request);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    // Covers timeout, cancellation, a reader/spawn error and an oversize reply.
    // try_wait already reaped successfully exited children.
    if outcome.is_err() {
        let _ = child.kill();
        child.wait().context("Could not reap the OneTagger process")?;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const ARTIST: &str = "L'Été & 東京";
    const TITLE: &str = "Café \"Blue\" (Dub Mix)";
    const MBID: &str = "5fb41d35-c38e-48de-98bf-c500d48bd33f";

    fn request() -> Request<'static> {
        Request {
            artist: ARTIST,
            title: TITLE,
        }
    }

    fn matched() -> Value {
        json!({"schema":1,"provider":"MusicBrainz","status":"matched","candidates":[{
            "recording_id":MBID,"artist":"l'été &   東京","title":"CAFÉ \"Blue\" (Dub Mix)",
            "genres":["House","house","Deep House"],"styles":[],"label":""}]})
    }

    fn parse(value: &Value) -> Result<Option<MatchTags>> {
        decode(&serde_json::to_vec(value)?, &request())
    }

    #[test]
    fn accepts_one_exact_recording_and_deduplicates_genres() {
        let tags = parse(&matched()).unwrap().unwrap();
        assert_eq!(tags.genres, ["House", "Deep House"]);
        assert_eq!(tags.provider, PROVIDER);
        assert_eq!(tags.recording_id, MBID);
        let mut empty = matched();
        empty["candidates"][0]["genres"] = json!([]);
        assert!(parse(&empty).unwrap().is_none());
        for status in ["no_match", "ambiguous"] {
            assert!(
                parse(&json!({"schema":1,"provider":PROVIDER,"status":status,"candidates":[]}))
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn rejects_other_editions_ambiguous_success_and_unverified_responses() {
        for (field, value) in [
            ("recording_id", json!("bad-id")),
            ("recording_id", json!("00000000-0000-0000-0000-000000000000")),
            ("artist", json!("Other Artist")),
            ("title", json!("Café \"Blue\" (Radio Mix)")),
            ("genres", json!(["bad\nvalue"])),
            ("genres", json!(vec!["house"; 65])),
        ] {
            let mut reply = matched();
            reply["candidates"][0][field] = value;
            assert!(parse(&reply).is_err(), "{field}");
        }
        let mut reply = matched();
        reply["candidates"]
            .as_array_mut()
            .unwrap()
            .push(matched()["candidates"][0].clone());
        assert!(parse(&reply).is_err());
        for (field, value) in [
            ("provider", json!("unknown")),
            ("schema", json!(2)),
            ("status", json!("partial")),
        ] {
            let mut reply = matched();
            reply[field] = value;
            assert!(parse(&reply).is_err());
        }
        assert!(decode(&vec![b' '; MAX_REPLY_BYTES + 1], &request()).is_err());
    }

    #[cfg(unix)]
    fn helper(directory: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("cli with spaces");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn passes_only_unicode_metadata_on_stdin_to_the_selected_command() {
        let temp = tempfile::tempdir().unwrap();
        let payload = Request {
            artist: "DJ `test` $(test) & 音楽",
            title: TITLE,
        };
        let mut reply = matched();
        reply["candidates"][0]["artist"] = json!(payload.artist);
        let cli = helper(temp.path(), &format!(
            "test \"$#\" = 2 && test \"$1\" = lookup && test \"$2\" = --json || exit 9\ncat > \"$0.input\"\ncat <<'END_JSON'\n{}\nEND_JSON", reply));
        let tags = lookup(&cli, payload.artist, payload.title, &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(tags.genres.len(), 2);
        let input: Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("cli with spaces.input")).unwrap())
                .unwrap();
        assert_eq!(input, serde_json::to_value(payload).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_cli_throttling_timeout_and_cancel_keep_distinct_messages() {
        let temp = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        for (body, reason) in [
            ("exit 2", "does not support"),
            ("exit 3", "asked OneTagger to pause"),
            ("yes x | head -c 70000", "exceeds 64 KiB"),
            ("exit 1", "could not complete"),
        ] {
            let err = lookup(&helper(temp.path(), body), ARTIST, TITLE, &cancel).unwrap_err();
            assert!(err.to_string().contains(reason), "{err}");
        }
        let cli = helper(temp.path(), "exec sleep 30");
        assert!(
            lookup_with_timeout(&cli, ARTIST, TITLE, &cancel, Duration::from_millis(100))
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(lookup(&cli, ARTIST, TITLE, &cancel)
                .unwrap_err()
                .to_string()
                .contains("cancelled"));
        });
    }
}
