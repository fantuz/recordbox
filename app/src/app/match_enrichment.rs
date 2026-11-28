//! Optional provider evidence belongs to the private catalog. The worker only sees
//! the chosen artist/title; the UI applies replies to their original catalog rows.
use super::App;
use crate::{db::Db, model::Track, onetagger::{MatchTags, Provider, ProviderError}};
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

    /// Guard the catalog identity and merge from the current database snapshot.
    /// Concurrent writers to the same library can fill different sources in either order.
    fn save(&self, db: &mut Db, provider: Provider, tags: &MatchTags) -> Result<Option<(Vec<MatchTags>, bool)>> {
        anyhow::ensure!(tags.provider == provider.name(), "Lookup returned a different provider");
        let tx = db.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ok: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks
            WHERE id=?1 AND path=?2 AND artist=?3 AND title=?4 AND mix_name=?5)",
            rusqlite::params![self.id, self.path, self.artist, self.title, self.mix_name], |r| r.get(0))?;
        if !ok { return Ok(None); }
        let mut existing = load_evidence(&tx, Some(self.id))?.remove(&self.id)
            .map(|(_, tags)| tags).unwrap_or_default();
        if has_provider(&existing, provider) {
            tx.commit()?;
            return Ok(Some((existing, false)));
        }
        let json = serde_json::to_string(tags)?;
        if provider == Provider::MusicBrainz {
            // Older versions continue to read their original single-source row.
            tx.execute("INSERT INTO match_enrichment(track_id,path,artist,title,mix_name,evidence)
                VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(track_id) DO UPDATE SET
                path=excluded.path, artist=excluded.artist, title=excluded.title,
                mix_name=excluded.mix_name, evidence=excluded.evidence",
                rusqlite::params![self.id, self.path, self.artist, self.title, self.mix_name, json])?;
        } else {
            tx.execute("INSERT INTO match_enrichment_sources(track_id,path,artist,title,mix_name,evidence,provider)
                VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(track_id,provider) DO UPDATE SET
                path=excluded.path, artist=excluded.artist, title=excluded.title,
                mix_name=excluded.mix_name, evidence=excluded.evidence",
                rusqlite::params![self.id, self.path, self.artist, self.title, self.mix_name, json, provider.name()])?;
        }
        tx.commit()?;
        existing.push(tags.clone());
        Ok(Some((existing, true)))
    }
}

type Evidence = HashMap<i64, (Identity, Vec<MatchTags>)>;

fn has_provider(tags: &[MatchTags], provider: Provider) -> bool {
    tags.iter().any(|row| row.provider == provider.name())
}

struct Request {
    key: Identity,
    missing: Vec<Provider>,
}

fn selected_query(t: &Track, sources: &[Provider], cached: Option<&(Identity, Vec<MatchTags>)>) -> Option<Request> {
    if t.artist.trim().is_empty() || t.title.trim().is_empty() { return None; }
    let saved = cached.filter(|(key, _)| key.matches(t)).map_or(&[][..], |(_, tags)| tags.as_slice());
    let missing: Vec<_> = sources.iter().copied().filter(|p| !has_provider(saved, *p)).collect();
    (!missing.is_empty()).then(|| Request { key: Identity::new(t), missing })
}

fn load_evidence(conn: &rusqlite::Connection, id: Option<i64>) -> Result<Evidence> {
    let mut sql = String::from("SELECT e.track_id,e.path,e.artist,e.title,e.mix_name,e.evidence,e.provider
        FROM (SELECT track_id,path,artist,title,mix_name,evidence,'MusicBrainz' AS provider FROM match_enrichment
        UNION ALL SELECT track_id,path,artist,title,mix_name,evidence,provider FROM match_enrichment_sources) e
        JOIN tracks t ON t.id=e.track_id AND e.path=t.path AND e.artist=t.artist AND e.title=t.title AND e.mix_name=t.mix_name");
    if id.is_some() { sql.push_str(" WHERE e.track_id=?1"); }
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(id))?;
    let mut out: Evidence = HashMap::new();
    while let Some(row) = rows.next()? {
        let Ok(source) = row.get_ref(6)?.as_str() else { continue };
        let Ok(json) = row.get_ref(5)?.as_str() else { continue };
        let Ok(mut tags) = serde_json::from_str::<MatchTags>(json) else { continue };
        if tags.provider.is_empty() && source == Provider::MusicBrainz.name() {
            tags.provider = source.to_owned();
        }
        if tags.provider != source || !Provider::ALL.iter().any(|p| p.name() == source)
            || (tags.genres.is_empty() && tags.styles.is_empty() && tags.label.is_empty()) { continue; }
        let key = Identity { id: row.get(0)?, path: row.get(1)?, artist: row.get(2)?, title: row.get(3)?, mix_name: row.get(4)? };
        out.entry(key.id).or_insert_with(|| (key, Vec::new())).1.push(tags);
    }
    Ok(out)
}

enum Event {
    Started(Provider, String),
    Hit(Identity, Provider, MatchTags),
    Checked(usize),
    Paused(Provider, String),
    End(Report),
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
    cached: [usize; Provider::ALL.len()],
    stale: usize,
    sources: Vec<Provider>,
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

// Each provider has its own pacing gate within this process. The gap starts on
// completion, including when the CLI fetched a second resource for a release.
static LAST_FINISHED: [Mutex<Option<Instant>>; Provider::ALL.len()] = [const { Mutex::new(None) }; Provider::ALL.len()];

#[derive(Default)]
struct ProviderStats {
    hits: usize,
    misses: usize,
    skipped: usize,
    paused: Option<String>,
}
fn provider_index(provider: Provider) -> usize {
    Provider::ALL.iter().position(|p| *p == provider).expect("registered provider")
}

struct ProviderTurn<'a>(std::sync::MutexGuard<'a, Option<Instant>>);
impl Drop for ProviderTurn<'_> {
    fn drop(&mut self) { *self.0 = Some(Instant::now()); }
}

enum Turn<T> {
    Ready(T),
    Wait(Duration),
}

fn provider_turn(last: &Mutex<Option<Instant>>, gap: Duration) -> Turn<ProviderTurn<'_>> {
    let slot = match last.try_lock() {
        Ok(slot) => slot,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Turn::Wait(Duration::from_millis(100)),
    };
    let remaining = slot.map(|t| gap.saturating_sub(t.elapsed())).unwrap_or_default();
    if remaining.is_zero() { Turn::Ready(ProviderTurn(slot)) } else { Turn::Wait(remaining) }
}

struct Report {
    stats: Vec<ProviderStats>,
    attempted_tracks: usize,
    blocked_tracks: usize,
}

fn run_batch<T>(
    requests: Vec<Request>, sources: &[Provider], cancel: &AtomicBool,
    mut send: impl FnMut(Event) -> bool, mut reserve: impl FnMut(Provider) -> Turn<T>,
    mut lookup: impl FnMut(Provider, &str, &str) -> Result<Option<MatchTags>>,
) {
    let mut groups: Vec<Vec<Request>> = Vec::new();
    let mut index = HashMap::new();
    for req in requests {
        let i = *index.entry((req.key.artist.clone(), req.key.lookup_title())).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[i].push(req);
    }
    let mut stats: Vec<ProviderStats> = Provider::ALL.iter().map(|_| ProviderStats::default()).collect();
    let mut attempted = HashSet::new();
    let mut blocked = 0;
    'batch: for group in groups {
        let first = &group[0].key;
        let title = first.lookup_title();
        // Prefer the selected order among available providers. A busy provider must
        // not hold up another source for this recording while its own clock runs.
        let mut pending = Vec::new();
        for &provider in sources {
            if !group.iter().any(|r| r.missing.contains(&provider)) { continue; }
            let stat = &mut stats[provider_index(provider)];
            if stat.paused.is_some() { stat.skipped += 1; } else { pending.push(provider); }
        }
        while !pending.is_empty() {
            if cancel.load(Ordering::Acquire) { break 'batch; }
            let mut wait = Duration::MAX;
            let next = pending.iter().enumerate().find_map(|(i, &p)| match reserve(p) {
                Turn::Ready(turn) => Some((i, turn)),
                Turn::Wait(d) => { wait = wait.min(d); None }
            });
            let Some((i, turn)) = next else {
                thread::park_timeout(wait);
                continue;
            };
            let provider = pending.remove(i);
            if cancel.load(Ordering::Acquire) || !send(Event::Started(provider, title.clone())) { break 'batch; }
            let needs_provider = |req: &&Request| req.missing.contains(&provider);
            attempted.extend(group.iter().filter(needs_provider).map(|req| req.key.id));
            let result = lookup(provider, &first.artist, &title);
            drop(turn);
            if cancel.load(Ordering::Acquire) { break 'batch; }
            let stat = &mut stats[provider_index(provider)];
            match result {
                Ok(Some(tags)) => {
                    stat.hits += 1;
                    for req in group.iter().filter(needs_provider) {
                        if cancel.load(Ordering::Acquire) || !send(Event::Hit(req.key.clone(), provider, tags.clone())) {
                            break 'batch;
                        }
                    }
                }
                Ok(None) => stat.misses += 1,
                Err(error) => {
                    let reason = match error.downcast_ref::<ProviderError>() {
                        Some(ProviderError::RateLimited) => "rate limited".to_owned(),
                        Some(ProviderError::Unsupported) => "unsupported by this OneTagger CLI".to_owned(),
                        _ => error.to_string(),
                    };
                    stat.paused = Some(reason.clone());
                    if !send(Event::Paused(provider, reason)) { break 'batch; }
                }
            }
        }
        blocked += group.iter().filter(|r| !attempted.contains(&r.key.id)).count();
        if !send(Event::Checked(group.len())) { break; }
    }
    send(Event::End(Report { stats, attempted_tracks: attempted.len(), blocked_tracks: blocked }));
}

impl App {
    pub(crate) fn match_tags(&self, id: i64) -> &[MatchTags] {
        self.match_enrichment.evidence.get(&id).filter(|(key, _)| self.track(id).is_some_and(|t| key.matches(t)))
            .map_or(&[], |(_, tags)| tags.as_slice())
    }

    pub(super) fn reload_match_evidence(&mut self) {
        match load_evidence(&self.db.conn, None) {
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

    pub(crate) fn queue_match_enrichment(&mut self, ids: &[i64], cli: &Path, providers: &[Provider]) {
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

        let mut sources = Vec::new();
        for &provider in providers {
            if !sources.contains(&provider) { sources.push(provider); }
        }
        if sources.is_empty() {
            self.match_enrichment.status = "Choose a lookup provider.".into();
            return;
        }
        let mut seen = HashSet::new();
        let mut skipped = 0;
        let mut cached = [0; Provider::ALL.len()];
        let mut requests = Vec::new();
        for &id in ids {
            if !seen.insert(id) { continue; }
            for &p in &sources {
                if has_provider(self.match_tags(id), p) { cached[provider_index(p)] += 1; }
            }
            let query = self.track(id).and_then(|t| selected_query(t, &sources, self.match_enrichment.evidence.get(&id)));
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
        let selected = sources.clone();
        match thread::Builder::new().name("onetagger-lookup".into()).spawn(move || {
            run_batch(requests, &selected, &flag, |event| {
                let delivered = tx.send(event).is_ok();
                ctx.request_repaint_of(eframe::egui::ViewportId::ROOT);
                delivered
            }, |p| provider_turn(&LAST_FINISHED[provider_index(p)], p.gap()),
               |p, artist, title| crate::onetagger::lookup_provider(&cli, artist, title, p, &flag));
        }) {
            Ok(thread) => {
                let e = &mut self.match_enrichment;
                e.worker = Some(Worker { rx, cancel, thread });
                e.status = format!("OneTagger lookup queued for {total} track(s).");
                e.total = total;
                e.saved = 0;
                e.checked = 0;
                e.eligible_skips = skipped;
                e.sources = sources;
                e.cached = cached;
                e.stale = 0;
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
            Ok(Event::Started(provider, title)) if !cancelled => {
                let e = &mut self.match_enrichment;
                e.status = format!("OneTagger · {} · track {} of {} · {title}", provider.name(), e.checked + 1, e.total);
            }
            Ok(Event::Hit(key, provider, tags)) if !cancelled => {
                if self.track(key.id).is_some_and(|t| key.matches(t)) {
                    match key.save(&mut self.db, provider, &tags) {
                        Ok(Some((saved, fresh))) => {
                            self.match_enrichment.evidence.insert(key.id, (key, saved));
                            self.match_enrichment.saved += usize::from(fresh);
                            self.ext.panel_cache = [None, None];
                            self.list_dirty = true;
                        }
                        Ok(None) => self.match_enrichment.stale += 1,
                        Err(e) => {
                            self.match_enrichment.error = Some(e.to_string());
                            self.cancel_match_enrichment();
                        }
                    }
                } else {
                    self.match_enrichment.stale += 1;
                }
            }
            Ok(Event::Checked(n)) if !cancelled => self.match_enrichment.checked += n,
            Ok(Event::Paused(provider, why)) if !cancelled => {
                self.match_enrichment.status = format!("{} paused for this lookup · {why}", provider.name());
            }
            Ok(Event::End(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                let e = &mut self.match_enrichment;
                let report = received.ok().and_then(|ev| if let Event::End(r) = ev { Some(r) } else { None });
                let reason = e.error.take().unwrap_or_else(|| {
                    if cancelled { "Cancelled".into() }
                    else if report.is_none() { "Worker stopped unexpectedly".into() }
                    else if report.as_ref().is_some_and(|r| r.stats.iter().any(|s| s.paused.is_some())) { "Finished with provider pauses".into() }
                    else { "Complete".into() }
                });
                e.worker.take();
                e.status = format!("OneTagger · {reason}. Saved {} source records. Skipped {} ineligible or fully cached tracks.", e.saved, e.eligible_skips);
                if let Some(report) = report {
                    use std::fmt::Write;
                    let _ = write!(e.status, " Attempted {} of {} eligible tracks. {} blocked by provider pauses.", report.attempted_tracks, e.total, report.blocked_tracks);
                    if e.stale > 0 { let _ = write!(e.status, " Discarded {} results for changed or removed tracks.", e.stale); }
                    for &provider in &e.sources {
                        let idx = provider_index(provider);
                        let stat = &report.stats[idx];
                        let _ = write!(e.status, " {} · {} hits, {} misses, {} cached tracks", provider.name(), stat.hits, stat.misses, e.cached[idx]);
                        if let Some(reason) = &stat.paused {
                            let _ = write!(e.status, ", paused ({reason}), {} later queries skipped", stat.skipped);
                        }
                        e.status.push('.');
                    }
                }
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

    fn request(key: Identity, providers: &[Provider]) -> Request {
        Request { key, missing: providers.to_vec() }
    }

    fn provider_tags(provider: Provider) -> MatchTags {
        MatchTags { provider: provider.name().into(), genres: vec!["House".into()], ..MatchTags::default() }
    }

    #[test]
    fn offline_track_is_eligible_and_saved_evidence_respects_edits_and_deletion() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.db");
        let mut db = Db::open(&path)?;
        let mut t = song("/detached/disc.flac", "Title");
        t.id = db.insert_track(&t)?;
        let key = selected_query(&t, &Provider::ALL, None).expect("catalog text is enough").key;
        for p in Provider::ALL {
            assert!(key.save(&mut db, p, &provider_tags(p))?.unwrap().1);
        }
        drop(db);
        let mut db = Db::open(&path)?;
        let evidence = load_evidence(&db.conn, None)?;
        assert!(selected_query(&t, &Provider::ALL, evidence.get(&t.id)).is_none());
        assert_eq!(evidence[&t.id].1.len(), 3);
        let local = db.track(t.id).unwrap();
        assert_eq!((local.genre.as_str(), local.label.as_str()), ("Library genre", "Library label"));
        for column in ["artist", "title", "mix_name", "path"] {
            let previous: String = db.conn.query_row(&format!("SELECT {column} FROM tracks WHERE id=?1"), [t.id], |r| r.get(0))?;
            db.conn.execute(&format!("UPDATE tracks SET {column}='changed' WHERE id=?1"), [t.id])?;
            assert!(key.save(&mut db, Provider::Deezer, &provider_tags(Provider::Deezer))?.is_none());
            assert!(load_evidence(&db.conn, None)?.is_empty());
            let q = selected_query(&db.track(t.id).unwrap(), &Provider::ALL, evidence.get(&t.id)).unwrap();
            assert_eq!(q.missing, Provider::ALL);
            db.conn.execute(&format!("UPDATE tracks SET {column}=?1 WHERE id=?2"), rusqlite::params![previous, t.id])?;
        }
        db.delete_track(t.id)?;
        assert!(key.save(&mut db, Provider::MusicBrainz, &provider_tags(Provider::MusicBrainz))?.is_none());
        for table in ["match_enrichment","match_enrichment_sources"] {
            assert_eq!(db.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0))?, 0);
        }
        Ok(())
    }

    #[test]
    fn concurrent_writers_add_missing_sources_without_replacing_the_legacy_evidence() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("library.db");
        let mut db = Db::open(&path)?;
        let mut t = song("/old.flac", "Original");
        t.id = db.insert_track(&t)?;
        let key = Identity::new(&t);
        // A literal v0.9.1 payload has neither the release fields nor a provider-scoped key.
        let legacy = r#"{"provider":"MusicBrainz","genres":["house"],"recording_id":"old-recording","label":"Existing label"}"#;
        db.conn.execute("INSERT INTO match_enrichment(track_id,path,artist,title,mix_name,evidence) VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![key.id, key.path, key.artist, key.title, key.mix_name, legacy])?;
        let current = load_evidence(&db.conn, None)?;
        assert_eq!(selected_query(&t, &Provider::ALL, current.get(&t.id)).unwrap().missing, [Provider::Itunes, Provider::Deezer]);

        let mut a = Db::open(&path)?;
        let mut b = Db::open(&path)?;
        let barrier = std::sync::Barrier::new(2);
        let write = |db: &mut Db, provider| {
            barrier.wait();
            key.save(db, provider, &provider_tags(provider))
        };
        thread::scope(|s| -> Result<()> {
            let one = s.spawn(|| write(&mut a, Provider::Itunes));
            let two = s.spawn(|| write(&mut b, Provider::Deezer));
            let (first, second) = (one.join().unwrap()?.unwrap(), two.join().unwrap()?.unwrap());
            assert!(first.1 && second.1);
            let mut lengths = [first.0.len(), second.0.len()];
            lengths.sort();
            assert_eq!(lengths, [2, 3]);
            Ok(())
        })?;
        let (all, inserted) = key.save(&mut db, Provider::MusicBrainz, &provider_tags(Provider::MusicBrainz))?.unwrap();
        assert!(!inserted);
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().find(|e| e.provider == "MusicBrainz").unwrap().recording_id, "old-recording");
        assert_eq!(db.conn.query_row("SELECT evidence FROM match_enrichment WHERE track_id=?1", [key.id], |r| r.get::<_, String>(0))?, legacy);
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
        let sources = [Provider::MusicBrainz];
        run_batch(vec![request(extended, &sources), request(Identity::new(&t), &sources)], &sources, &AtomicBool::new(false),
            |_| true, |_| Turn::Ready(()), |_, _, title| { calls.push(title.to_owned()); Ok(None) });
        assert_eq!(calls, ["Track (Extended Mix)", "Track (Radio Edit)"]);
    }

    #[test]
    fn duplicate_queries_share_provider_calls_and_paused_services_leave_other_sources_running() {
        use Provider::*;
        let songs = [("one", "same"), ("two", "next"), ("three", "same"), ("four", "later")];
        let requests = songs.into_iter().enumerate().map(|(i,(path,title))| {
            let mut t = song(path, title);
            t.id = i as i64;
            // This row already has iTunes. The matching second copy needs it.
            request(Identity::new(&t), if i == 0 { &[MusicBrainz, Deezer] } else { &Provider::ALL })
        }).collect();
        let mut calls = vec![];
        let mut hits = vec![];
        let mut report = vec![];
        let mut checked = 0;
        run_batch(requests, &Provider::ALL, &AtomicBool::new(false), |event| {
            match event {
                Event::Hit(key, p, _) => hits.push((key.id, p)),
                Event::Checked(n) => checked += n,
                Event::End(r) => {
                    assert_eq!((r.attempted_tracks, r.blocked_tracks), (4, 0));
                    report = r.stats;
                }
                _ => ()
            }
            true
        }, |_| Turn::Ready(()), |p, _, title| {
            calls.push((p, title.to_owned()));
            match (p, title) {
                (MusicBrainz, _) => Err(ProviderError::RateLimited.into()),
                (Deezer, _) => Err(ProviderError::Unsupported.into()),
                (_, "next") => Ok(None),
                _ => Ok(Some(provider_tags(p))),
            }
        });
        assert_eq!(calls.iter().map(|(p,t)| (*p,t.as_str())).collect::<Vec<_>>(),
            [(MusicBrainz, "same"), (Itunes, "same"), (Deezer, "same"), (Itunes, "next"), (Itunes, "later")]);
        assert_eq!(hits, [(2, Itunes), (3, Itunes)]);
        assert_eq!(checked, 4);
        assert_eq!(report[provider_index(MusicBrainz)].skipped, 2);
        assert_eq!(report[provider_index(Deezer)].skipped, 2);
        assert_eq!((report[provider_index(Itunes)].hits, report[provider_index(Itunes)].misses), (2, 1));
    }

    #[test]
    fn cancellation_after_lookup_discards_the_reply_and_stops_before_the_next_source() {
        let cancel = AtomicBool::new(false);
        let mut checked = vec![];
        let mut hits = 0;
        let mut ended = false;
        run_batch(vec![request(Identity::new(&song("one","song")), &Provider::ALL)], &Provider::ALL, &cancel,
            |e| { match e { Event::Hit(..) => hits += 1, Event::End(_) => ended = true, _ => () }; true },
            |_| Turn::Ready(()), |p,_,_| {
                checked.push(p);
                cancel.store(true, Ordering::Release);
                Ok(Some(provider_tags(p)))
            });
        assert_eq!(checked, [Provider::MusicBrainz]);
        assert!(ended && hits == 0);
    }

    #[test]
    fn a_cooling_provider_yields_to_an_available_source_for_the_same_track() {
        let sources = [Provider::MusicBrainz, Provider::Itunes];
        let mut cooling = true;
        let mut calls = vec![];
        run_batch(vec![request(Identity::new(&song("/x", "song")), &sources)], &sources, &AtomicBool::new(false),
            |_| true, |p| {
                if p == Provider::MusicBrainz && std::mem::take(&mut cooling) { Turn::Wait(Duration::from_secs(60)) }
                else { Turn::Ready(()) }
            }, |p,_,_| { calls.push(p); Ok(None) });
        assert_eq!(calls, [Provider::Itunes, Provider::MusicBrainz]);
    }

    #[test]
    fn a_paused_only_provider_does_not_claim_remaining_tracks_were_attempted() {
        let sources = [Provider::MusicBrainz];
        let requests = [("first", 1), ("second", 2)].map(|(title, id)| {
            let mut t = song(title, title);
            t.id = id;
            request(Identity::new(&t), &sources)
        });
        let mut counts = None;
        run_batch(requests.into(), &sources, &AtomicBool::new(false), |e| {
            if let Event::End(r) = e { counts = Some((r.attempted_tracks, r.blocked_tracks)); }
            true
        }, |_| Turn::Ready(()), |_,_,_| Err(ProviderError::RateLimited.into()));
        assert_eq!(counts, Some((1, 1)));
    }

    #[test]
    fn rate_limit_carries_across_batches_and_distinct_provider_slots_are_independent() {
        let last = Mutex::new(None);
        let other = Mutex::new(None);
        let gap = Duration::from_millis(12);
        let Turn::Ready(first) = provider_turn(&last, gap) else { panic!("free slot") };
        assert!(matches!(provider_turn(&last, gap), Turn::Wait(_)));
        assert!(matches!(provider_turn(&other, gap), Turn::Ready(_)));
        drop(first);
        let finished = last.lock().unwrap().unwrap();
        while let Turn::Wait(d) = provider_turn(&last, gap) { thread::sleep(d); }
        assert!(finished.elapsed() >= gap);
    }
}
