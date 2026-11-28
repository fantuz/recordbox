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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Provider {
    MusicBrainz,
    Itunes,
    Deezer,
}

impl Provider {
    pub(crate) const ALL: [Self; 3] = [Self::MusicBrainz, Self::Itunes, Self::Deezer];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::MusicBrainz => PROVIDER,
            Self::Itunes => "iTunes",
            Self::Deezer => "Deezer",
        }
    }

    pub(crate) fn cli_id(self) -> &'static str {
        match self {
            Self::MusicBrainz => "musicbrainz",
            Self::Itunes => "itunes",
            Self::Deezer => "deezer",
        }
    }

    pub(crate) fn gap(self) -> Duration {
        Duration::from_secs(match self {
            Self::Itunes => 3,
            Self::Deezer | Self::MusicBrainz => 1,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderError {
    RateLimited,
    Unsupported,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RateLimited => "Provider asked OneTagger to pause",
            Self::Unsupported => "This executable does not support the requested lookup",
        })
    }
}

impl std::error::Error for ProviderError {}

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
    pub track_id: String,
    pub release_id: String,
    pub genre_scope: String,
}

impl MatchTags {
    pub(crate) fn source_url(&self) -> Option<String> {
        match self.provider.as_str() {
            PROVIDER if recording_id(&self.recording_id) => Some(format!(
                "https://musicbrainz.org/recording/{}",
                self.recording_id
            )),
            "iTunes" if catalog_id(&self.track_id) && catalog_id(&self.release_id) => {
                Some(format!(
                    "https://music.apple.com/us/album/{}?i={}",
                    self.release_id, self.track_id
                ))
            }
            "Deezer" if catalog_id(&self.track_id) => {
                Some(format!("https://www.deezer.com/track/{}", self.track_id))
            }
            _ => None,
        }
    }
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
    #[serde(default)]
    recording_id: String,
    #[serde(default)]
    track_id: String,
    #[serde(default)]
    release_id: String,
    #[serde(default)]
    genre_scope: String,
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

pub(crate) fn valid_query(artist: &str, title: &str) -> bool {
    valid_text(artist.trim()) && valid_text(title.trim())
}

fn identity(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
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

fn catalog_id(s: &str) -> bool {
    !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u64>().is_ok()
}

fn verified_tags(values: Vec<String>) -> Result<Vec<String>> {
    ensure!(values.len() <= 64, "OneTagger returned too many tag values");
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in values {
        ensure!(
            valid_text(&value),
            "OneTagger returned an invalid tag value"
        );
        let value = value.trim();
        if seen.insert(identity(value)) {
            result.push(value.to_owned());
        }
    }
    Ok(result)
}

fn decode(bytes: &[u8], request: &Request<'_>, provider: Provider) -> Result<Option<MatchTags>> {
    ensure!(
        bytes.len() <= MAX_REPLY_BYTES,
        "OneTagger lookup reply exceeds 64 KiB"
    );
    let reply: Reply = serde_json::from_slice(bytes).context("Invalid OneTagger lookup reply")?;
    let schema = if provider == Provider::MusicBrainz {
        1
    } else {
        2
    };
    ensure!(
        reply.schema == schema && reply.provider == provider.name(),
        "Unsupported OneTagger lookup reply"
    );
    match reply.status.as_str() {
        "no_match" | "ambiguous" => {
            ensure!(
                schema == 1 || reply.candidates.is_empty(),
                "Unconfirmed catalog result contained candidates"
            );
            return Ok(None);
        }
        "matched" => {}
        _ => bail!("Unknown OneTagger lookup status"),
    }
    ensure!(
        reply.candidates.len() == 1,
        "OneTagger marked a non-unique result as matched"
    );
    let hit = reply.candidates.into_iter().next().unwrap();
    ensure!(
        valid_text(&hit.artist)
            && valid_text(&hit.title)
            && identity(&hit.artist) == identity(request.artist)
            && identity(&hit.title) == identity(request.title),
        "OneTagger returned a different recording or edition"
    );
    let scope = match provider {
        Provider::MusicBrainz => {
            ensure!(
                recording_id(&hit.recording_id),
                "Invalid MusicBrainz recording ID"
            );
            "recording"
        }
        Provider::Itunes | Provider::Deezer => {
            let scope = if provider == Provider::Itunes {
                "track"
            } else {
                "release"
            };
            ensure!(
                catalog_id(&hit.track_id)
                    && catalog_id(&hit.release_id)
                    && hit.recording_id.is_empty(),
                "Invalid track or release ID for {}",
                provider.name()
            );
            ensure!(
                hit.genre_scope == scope
                    && hit.styles.is_empty()
                    && (provider == Provider::Deezer || hit.label.is_empty()),
                "Unexpected field or tag scope for {}",
                provider.name()
            );
            scope
        }
    };
    let genres = verified_tags(hit.genres)?;
    let styles = verified_tags(hit.styles)?;
    let label = hit.label.trim();
    ensure!(
        hit.label.is_empty() || valid_text(&hit.label),
        "OneTagger returned an invalid label"
    );
    Ok(
        (!genres.is_empty() || !styles.is_empty() || !label.is_empty()).then(|| MatchTags {
            genres,
            styles,
            label: label.to_owned(),
            provider: reply.provider,
            recording_id: hit.recording_id,
            track_id: hit.track_id,
            release_id: hit.release_id,
            genre_scope: scope.into(),
        }),
    )
}

fn check_exit(status: ExitStatus, provider: Provider) -> Result<()> {
    match status.code() {
        Some(0) => Ok(()),
        Some(2) => Err(anyhow::Error::new(ProviderError::Unsupported).context(format!("This executable does not support the requested {} lookup. Select a compatible OneTagger CLI.", provider.name()))),
        Some(3) => Err(anyhow::Error::new(ProviderError::RateLimited).context(format!("{} asked OneTagger to pause. Try this provider again later.", provider.name()))),
        _ => bail!("OneTagger could not complete the {} lookup ({status}).", provider.name()),
    }
}

/// The caller runs this on the enrichment worker. All send/receive waits are
/// bounded independently of the deck, audio analysis and GUI threads.
pub(crate) fn lookup_provider(
    cli: &Path,
    artist: &str,
    title: &str,
    provider: Provider,
    cancel: &AtomicBool,
) -> Result<Option<MatchTags>> {
    lookup_with_timeout(cli, artist, title, provider, cancel, TIMEOUT)
}

fn lookup_with_timeout(
    cli: &Path,
    artist: &str,
    title: &str,
    provider: Provider,
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
        valid_query(request.artist, request.title),
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
    cmd.args(["lookup", "--json"]);
    // Omitting the selector keeps the original MusicBrainz-only CLI compatible.
    if provider != Provider::MusicBrainz {
        cmd.args(["--provider", provider.cli_id()]);
    }
    cmd.current_dir(dir.path())
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
                let read = stdout
                    .take(MAX_REPLY_BYTES as u64 + 1)
                    .read_to_end(&mut bytes);
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
                    Err(mpsc::TryRecvError::Disconnected) => {
                        bail!("OneTagger lookup reader stopped")
                    }
                }
            }
            if let Some(status) = child.try_wait()? {
                check_exit(status, provider)?;
                if let Some(output) = &bytes {
                    return decode(output, &request, provider);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    // Covers timeout, cancellation, a reader/spawn error and an oversize reply.
    // try_wait already reaped successfully exited children.
    if outcome.is_err() {
        let _ = child.kill();
        child
            .wait()
            .context("Could not reap the OneTagger process")?;
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
        decode(
            &serde_json::to_vec(value)?,
            &request(),
            Provider::MusicBrainz,
        )
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
            assert!(parse(
                &json!({"schema":1,"provider":PROVIDER,"status":status,"candidates":[]})
            )
            .unwrap()
            .is_none());
        }
    }

    #[test]
    fn rejects_other_editions_ambiguous_success_and_unverified_responses() {
        for (field, value) in [
            ("recording_id", json!("bad-id")),
            (
                "recording_id",
                json!("00000000-0000-0000-0000-000000000000"),
            ),
            ("artist", json!("Other Artist")),
            ("title", json!("Café \"Blue\" (Radio Mix)")),
            ("genres", json!(["bad\nvalue"])),
            ("genres", json!(["house\n"])),
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
        assert!(decode(
            &vec![b' '; MAX_REPLY_BYTES + 1],
            &request(),
            Provider::MusicBrainz
        )
        .is_err());
    }

    #[test]
    fn store_evidence_keeps_the_id_and_scope_of_each_service() {
        for (provider, scope) in [(Provider::Itunes, "track"), (Provider::Deezer, "release")] {
            let bytes = |item: Value| {
                serde_json::to_vec(&json!({"schema":2,"provider":provider.name(),"status":"matched","candidates":[item]})).unwrap()
            };
            let candidate = json!({"artist":ARTIST,"title":TITLE,"track_id":"154","release_id":"28","genre_scope":scope,"genres":["House"]});
            let decode_item = |item| decode(&bytes(item), &request(), provider);
            let found = decode_item(candidate.clone()).unwrap().unwrap();
            assert_eq!(
                (
                    found.track_id.as_str(),
                    found.release_id.as_str(),
                    found.genre_scope.as_str()
                ),
                ("154", "28", scope)
            );
            assert!(found.recording_id.is_empty() && found.source_url().unwrap().contains("154"));
            for (key, value) in [
                ("release_id", "0"),
                ("track_id", "1/2"),
                ("genre_scope", "recording"),
                ("recording_id", MBID),
            ] {
                let mut bad = candidate.clone();
                bad[key] = json!(value);
                assert!(decode_item(bad).is_err(), "{key}");
            }
            assert!(decode(&bytes(candidate), &request(), Provider::MusicBrainz).is_err());
        }
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
        let tags = lookup_provider(
            &cli,
            payload.artist,
            payload.title,
            Provider::MusicBrainz,
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tags.genres.len(), 2);
        let input: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("cli with spaces.input")).unwrap(),
        )
        .unwrap();
        assert_eq!(input, serde_json::to_value(payload).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn catalog_provider_is_explicit_and_query_contains_only_artist_and_title() {
        let dir = tempfile::tempdir().unwrap();
        for provider in [Provider::Itunes, Provider::Deezer] {
            let reply =
                json!({"schema":2,"provider":provider.name(),"status":"no_match","candidates":[]});
            let script = format!(
                r#"test "$#" = 4 && test "$1" = lookup && test "$2" = --json && test "$3" = --provider && test "$4" = {} || exit 9
cat > "$0.input"
echo '{}'"#,
                provider.cli_id(),
                reply
            );
            let result = lookup_provider(
                &helper(dir.path(), &script),
                ARTIST,
                TITLE,
                provider,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(result.is_none());
            let input: Value = serde_json::from_slice(
                &std::fs::read(dir.path().join("cli with spaces.input")).unwrap(),
            )
            .unwrap();
            assert_eq!(input, serde_json::to_value(request()).unwrap());
        }
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
            let err = lookup_provider(
                &helper(temp.path(), body),
                ARTIST,
                TITLE,
                Provider::MusicBrainz,
                &cancel,
            )
            .unwrap_err();
            assert!(err.to_string().contains(reason), "{err}");
        }
        let cli = helper(temp.path(), "exec sleep 30");
        assert!(lookup_with_timeout(
            &cli,
            ARTIST,
            TITLE,
            Provider::MusicBrainz,
            &cancel,
            Duration::from_millis(100)
        )
        .unwrap_err()
        .to_string()
        .contains("timed out"));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(
                lookup_provider(&cli, ARTIST, TITLE, Provider::MusicBrainz, &cancel)
                    .unwrap_err()
                    .to_string()
                    .contains("cancelled")
            );
        });
    }
}
