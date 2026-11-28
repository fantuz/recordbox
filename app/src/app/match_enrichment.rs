//! Optional provider evidence belongs to the private catalog. The worker only sees
//! the chosen artist/title; the UI applies replies to their original catalog rows.
use super::App;
use crate::{db::Db, model::Track, onetagger::MatchTags};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(crate) const MATCH_LOOKUP_LIMIT: usize = 100;

#[derive(Clone)]
struct Identity {
    id: i64,
    path: String,
    artist: String,
    title: String,
    mix_name: String,
}

impl Identity {
    fn new(t: &Track) -> Self {
        Self { id: t.id, path: t.path.clone(), artist: t.artist.clone(), title: t.title.clone(), mix_name: t.mix_name.clone() }
    }

    fn matches(&self, t: &Track) -> bool {
        self.id == t.id && self.artist == t.artist && self.title == t.title && self.mix_name == t.mix_name && self.path == t.path
    }

    fn lookup_title(&self) -> String {
        let title = self.title.trim();
        let mix = self.mix_name.trim();
        let mix = mix.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
            .or_else(|| mix.strip_prefix('[').and_then(|s| s.strip_suffix(']'))).unwrap_or(mix).trim();
        if mix.is_empty() { return title.to_owned(); }
        let lower = title.to_lowercase();
        let qualifier = mix.to_lowercase();
        if lower.contains(&format!("({qualifier})")) || lower.contains(&format!("[{qualifier}]"))
            || [" - ", " – ", " — "].iter().any(|sep| lower.ends_with(&format!("{sep}{qualifier}"))) {
            title.to_owned()
        } else {
            format!("{title} ({mix})")
        }
    }

    fn save(&self, db: &Db, tags: &MatchTags) -> Result<bool> {
        // Recheck the persisted identity in the write itself. Another instance can
        // edit or delete this row between the provider response and the UI frame.
        let count = db.conn.execute("INSERT INTO match_enrichment(track_id,path,artist,title,mix_name,evidence)
            SELECT id,path,artist,title,mix_name,?6 FROM tracks WHERE id=?1 AND path=?2 AND artist=?3 AND title=?4 AND mix_name=?5
            ON CONFLICT(track_id) DO UPDATE SET path=excluded.path, artist=excluded.artist,
                title=excluded.title, mix_name=excluded.mix_name, evidence=excluded.evidence",
            rusqlite::params![self.id, self.path, self.artist, self.title, self.mix_name, serde_json::to_string(tags)?])?;
        Ok(count == 1)
    }
}

type Evidence = HashMap<i64, (Identity, MatchTags)>;

fn selected_query(t: &Track, cached: Option<&(Identity, MatchTags)>) -> Option<Identity> {
    if t.artist.trim().is_empty() || t.title.trim().is_empty() || cached.is_some_and(|(key, _)| key.matches(t)) {
        return None;
    }
    // A cloud-only or disconnected track needs no local file for a catalog text lookup.
    Some(Identity::new(t))
}

fn load_evidence(db: &Db) -> Result<Evidence> {
    let mut stmt = db.conn.prepare("SELECT e.track_id,e.path,e.artist,e.title,e.mix_name,e.evidence
        FROM match_enrichment e JOIN tracks t ON t.id=e.track_id
        AND e.path=t.path AND e.artist=t.artist AND e.title=t.title AND e.mix_name=t.mix_name")?;
    let mut rows = stmt.query([])?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next()? {
        let key = Identity { id: row.get(0)?, path: row.get(1)?, artist: row.get(2)?, title: row.get(3)?, mix_name: row.get(4)? };
        if let Ok(tags) = serde_json::from_str::<MatchTags>(row.get_ref(5)?.as_str()?) {
            out.insert(key.id, (key, tags));
        }
    }
    Ok(out)
}

enum Event {
    Started(String),
    Reply(Identity, Result<Option<MatchTags>>),
    End,
}

struct Worker {
    rx: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Worker {
    fn stop(&self) {
        self.cancel.store(true, Ordering::Release);
        self.thread.thread().unpark();
    }
}

#[derive(Default)]
pub(super) struct Enrichment {
    evidence: Evidence,
    worker: Option<Worker>,
    status: String,
    total: usize,
    checked: usize,
    saved: usize,
    eligible_skips: usize,
    error: Option<String>,
}

impl Drop for Enrichment {
    fn drop(&mut self) {
        // Allow the CLI adapter to terminate/reap its child before the application exits.
        if let Some(worker) = self.worker.take() {
            worker.stop();
            let _ = worker.thread.join();
        }
    }
}

// The CLI may make a details request after its initial search. Space consecutive
// processes from the preceding completion and serialize calls across app instances.
static LAST_FINISHED: Mutex<Option<Instant>> = Mutex::new(None);
struct ProviderTurn<'a>(std::sync::MutexGuard<'a, Option<Instant>>);
impl Drop for ProviderTurn<'_> {
    fn drop(&mut self) { *self.0 = Some(Instant::now()); }
}

fn reserve_turn<'a>(last: &'a Mutex<Option<Instant>>, gap: Duration, cancel: &AtomicBool) -> Option<ProviderTurn<'a>> {
    while !cancel.load(Ordering::Acquire) {
        let slot = match last.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                thread::park_timeout(Duration::from_millis(100));
                continue;
            }
        };
        let remaining = slot.map(|t| gap.saturating_sub(t.elapsed())).unwrap_or_default();
        if remaining.is_zero() { return Some(ProviderTurn(slot)); }
        drop(slot);
        thread::park_timeout(remaining);
    }
    None
}

fn run_batch<T>(
    requests: Vec<Identity>, cancel: &AtomicBool,
    mut send: impl FnMut(Event) -> bool, mut reserve: impl FnMut() -> Option<T>,
    mut lookup: impl FnMut(&str, &str) -> Result<Option<MatchTags>>,
) {
    let mut groups: Vec<Vec<Identity>> = Vec::new();
    let mut index = HashMap::new();
    for query in requests {
        let i = *index.entry((query.artist.clone(), query.lookup_title())).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[i].push(query);
    }
    'batch: for group in groups {
        let first = &group[0];
        let title = first.lookup_title();
        if cancel.load(Ordering::Acquire) { break; }
        let Some(turn) = reserve() else { break };
        if cancel.load(Ordering::Acquire) || !send(Event::Started(title.clone())) { break; }
        let result = lookup(&first.artist, &title);
        drop(turn);
        match result {
            Ok(tags) => {
                for track in group {
                    if cancel.load(Ordering::Acquire) || !send(Event::Reply(track, Ok(tags.clone()))) {
                        break 'batch;
                    }
                }
            }
            Err(error) => {
                if let Some(track) = group.into_iter().next() { send(Event::Reply(track, Err(error))); }
                break;
            }
        }
    }
    send(Event::End);
}

impl App {
    pub(crate) fn match_tags(&self, id: i64) -> Option<&MatchTags> {
        let (identity, tags) = self.match_enrichment.evidence.get(&id)?;
        identity.matches(self.track(id)?).then_some(tags)
    }

    pub(super) fn reload_match_evidence(&mut self) {
        match load_evidence(&self.db) {
            Ok(evidence) => self.match_enrichment.evidence = evidence,
            Err(e) => {
                self.match_enrichment.evidence.clear();
                self.match_enrichment.status = format!("Could not load saved OneTagger matches. {e}");
            }
        }
        self.ext.panel_cache = [None, None];
    }

    pub(crate) fn match_enrichment_running(&self) -> bool { self.match_enrichment.worker.is_some() }
    pub(crate) fn match_enrichment_status(&self) -> String { self.match_enrichment.status.clone() }

    pub(crate) fn cancel_match_enrichment(&mut self) {
        if let Some(worker) = &self.match_enrichment.worker {
            worker.stop();
            self.match_enrichment.status = "Stopping OneTagger lookup…".into();
        }
    }

    pub(crate) fn queue_match_enrichment(&mut self, ids: &[i64], cli: &Path) {
        if self.match_enrichment_running() {
            self.toast("A OneTagger lookup is already running.");
            return;
        }
        if ids.len() > MATCH_LOOKUP_LIMIT {
            let msg = format!("Choose up to {MATCH_LOOKUP_LIMIT} tracks for one OneTagger lookup.");
            self.match_enrichment.status = msg.clone();
            self.toast(msg);
            return;
        }

        let mut seen = HashSet::new();
        let mut skipped = 0;
        let mut requests = Vec::new();
        for &id in ids {
            if !seen.insert(id) { continue; }
            let query = self.track(id).and_then(|t| selected_query(t, self.match_enrichment.evidence.get(&id)));
            if let Some(query) = query { requests.push(query); } else { skipped += 1; }
        }
        if requests.is_empty() {
            let msg = "No tracks to look up. Results are already saved, or the selected tracks lack an artist or title.";
            self.match_enrichment.status = msg.into();
            self.toast(msg);
            return;
        }

        let total = requests.len();
        let cli = cli.to_owned();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ctx = self.ctx.clone();
        match thread::Builder::new().name("onetagger-lookup".into()).spawn(move || {
            run_batch(requests, &flag, |event| {
                let delivered = tx.send(event).is_ok();
                ctx.request_repaint();
                delivered
            }, || reserve_turn(&LAST_FINISHED, Duration::from_secs(1), &flag),
               |artist, title| crate::onetagger::lookup(&cli, artist, title, &flag));
        }) {
            Ok(thread) => {
                let e = &mut self.match_enrichment;
                e.worker = Some(Worker { rx, cancel, thread });
                e.status = format!("OneTagger lookup queued for {total} track(s).");
                e.total = total;
                e.saved = 0;
                e.checked = 0;
                e.eligible_skips = skipped;
                e.error = None;
            }
            Err(error) => {
                let msg = format!("Could not start OneTagger lookup. {error}");
                self.match_enrichment.status = msg.clone();
                self.toast_err(msg);
            }
        }
    }

    pub(crate) fn poll_match_enrichment(&mut self) {
        let Some(worker) = &self.match_enrichment.worker else { return };
        let received = worker.rx.try_recv();
        let cancelled = worker.cancel.load(Ordering::Acquire);
        match received {
            Err(mpsc::TryRecvError::Empty) => return,
            Ok(Event::Started(title)) if !cancelled => {
                let e = &mut self.match_enrichment;
                e.status = format!("OneTagger · {} of {} · {title}", e.checked + 1, e.total);
            }
            Ok(Event::Reply(key, result)) if !cancelled => {
                self.match_enrichment.checked += 1;
                let outcome = result.and_then(|tags| {
                    if let Some(tags) = tags {
                        if self.track(key.id).is_some_and(|t| key.matches(t)) && key.save(&self.db, &tags)? {
                            self.match_enrichment.evidence.insert(key.id, (key, tags));
                            self.match_enrichment.saved += 1;
                            self.ext.panel_cache = [None, None];
                            self.list_dirty = true;
                        }
                    }
                    Ok(())
                });
                if let Err(error) = outcome {
                    self.match_enrichment.error = Some(error.to_string());
                    self.cancel_match_enrichment();
                }
            }
            Ok(Event::End) | Err(mpsc::TryRecvError::Disconnected) => {
                let e = &mut self.match_enrichment;
                let reason = e.error.take().unwrap_or_else(|| {
                    if cancelled { "Cancelled".into() }
                    else if received.is_err() { "Worker stopped unexpectedly".into() }
                    else { "Complete".into() }
                });
                e.worker.take();
                e.status = format!("OneTagger · {reason}. Checked {} of {}, saved {}, skipped {} before lookup.",
                    e.checked, e.total, e.saved, e.eligible_skips);
                self.toast(self.match_enrichment.status.clone());
                return;
            }
            _ => {}
        }
        // UI wakes on each worker message; drain any coalesced messages on the next frame.
        self.ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(path: &str, title: &str) -> Track {
        Track { path: path.into(), artist: "Artist".into(), title: title.into(), missing: true,
            genre: "Library genre".into(), label: "Library label".into(), ..Track::default() }
    }

    fn tags() -> MatchTags {
        MatchTags { genres: vec!["electronic".into()], label: "Provider label".into(),
            provider: crate::onetagger::PROVIDER.into(), ..MatchTags::default() }
    }

    #[test]
    fn offline_track_is_eligible_and_saved_evidence_respects_edits_and_deletion() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.db");
        let db = Db::open(&path)?;
        let mut t = song("/detached/disc.flac", "Title");
        t.id = db.insert_track(&t)?;
        let key = selected_query(&t, None).expect("catalog text is enough to look up an offline track");
        assert!(key.save(&db, &tags())?);
        drop(db);
        let db = Db::open(&path)?;
        let evidence = load_evidence(&db)?;
        assert!(selected_query(&t, evidence.get(&t.id)).is_none());
        assert_eq!(evidence[&t.id].1, tags());
        let local = db.track(t.id).unwrap();
        assert_eq!((local.genre.as_str(), local.label.as_str()), ("Library genre", "Library label"));
        for column in ["artist", "title", "mix_name", "path"] {
            let previous: String = db.conn.query_row(&format!("SELECT {column} FROM tracks WHERE id=?1"), [t.id], |r| r.get(0))?;
            db.conn.execute(&format!("UPDATE tracks SET {column}='changed' WHERE id=?1"), [t.id])?;
            assert!(!key.save(&db, &tags())?);
            assert!(load_evidence(&db)?.is_empty());
            assert!(selected_query(&db.track(t.id).unwrap(), evidence.get(&t.id)).is_some());
            db.conn.execute(&format!("UPDATE tracks SET {column}=?1 WHERE id=?2"), rusqlite::params![previous, t.id])?;
        }
        db.delete_track(t.id)?;
        assert!(!key.save(&db, &tags())?);
        assert_eq!(db.conn.query_row("SELECT COUNT(*) FROM match_enrichment", [], |r| r.get::<_, i64>(0))?, 0);
        Ok(())
    }

    #[test]
    fn separate_mix_title_is_sent_once_and_different_editions_stay_distinct() {
        let mut t = song("/track.flac", "Track");
        t.mix_name = "Extended Mix".into();
        assert_eq!(Identity::new(&t).lookup_title(), "Track (Extended Mix)");
        for title in ["Track (extended mix)", "Track [Extended Mix]", "Track – Extended Mix"] {
            t.title = title.into();
            assert_eq!(Identity::new(&t).lookup_title(), title);
        }
        t.title = "Track".into();
        let extended = Identity::new(&t);
        t.mix_name = "Radio Edit".into();
        assert!(!extended.matches(&t));
        let mut calls = Vec::new();
        run_batch(vec![extended, Identity::new(&t)], &AtomicBool::new(false), |_| true, || Some(()),
            |_, title| { calls.push(title.to_owned()); Ok(None) });
        assert_eq!(calls, ["Track (Extended Mix)", "Track (Radio Edit)"]);
    }

    #[test]
    fn duplicate_queries_share_one_call_and_provider_error_stops_remaining_requests() {
        let songs = [("one", "same"), ("two", "next"), ("three", "same"), ("four", "later")];
        let queries = songs.into_iter().enumerate().map(|(i,(path,title))| {
            let mut t = song(path, title); t.id = i as i64; Identity::new(&t)
        }).collect();
        let mut calls = vec![];
        let mut saved_ids = vec![];
        let mut errors = 0;
        run_batch(queries, &AtomicBool::new(false), |ev| {
            if let Event::Reply(row, reply) = ev {
                if reply.is_ok() { saved_ids.push(row.id); } else { errors += 1; }
            }
            true
        }, || Some(()), |_, title| {
            calls.push(title.to_owned());
            if title == "next" { anyhow::bail!("provider unavailable"); }
            Ok(Some(tags()))
        });
        assert_eq!(calls, ["same", "next"]);
        assert_eq!(saved_ids, [0, 2]);
        assert_eq!(errors, 1);
    }

    #[test]
    fn rate_limit_carries_across_batches_and_a_cancelled_slot_is_free() {
        let last = Mutex::new(None);
        let cancel = AtomicBool::new(false);
        let gap = Duration::from_millis(12);
        let first = reserve_turn(&last, gap, &cancel).unwrap();
        assert!(last.try_lock().is_err(), "a second provider call cannot overlap this one");
        // The provider may spend longer than the nominal start-to-start interval on the network.
        thread::sleep(gap);
        drop(first);
        let finished = last.lock().unwrap().unwrap();
        let second = reserve_turn(&last, gap, &cancel).unwrap();
        assert!(finished.elapsed() >= gap);
        drop(second);
        let timestamp = *last.lock().unwrap();
        cancel.store(true, Ordering::Release);
        assert!(reserve_turn(&last, gap, &cancel).is_none());
        assert_eq!(*last.lock().unwrap(), timestamp);
    }
}
