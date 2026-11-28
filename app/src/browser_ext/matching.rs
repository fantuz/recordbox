//! Local DJ recommendations. Required Related Tracks criteria decide eligibility before
//! ranking. Enrichment can supply additional evidence without writing to a track's tags.
use crate::app::{key_distance, App, RelatedRules};
use crate::model::Track;
use crate::onetagger::MatchTags;
use std::collections::HashSet;

fn year_of(t: &Track) -> Option<i32> {
    let valid = |y: &i32| (1901..=9999).contains(y);
    Some(t.year).filter(valid).or_else(|| t.release_date.get(..4)?.parse().ok().filter(valid))
}

pub(super) fn era_score(a: &Track, b: &Track) -> Option<f64> {
    let gap = year_of(a)?.abs_diff(year_of(b)?) as f64;
    Some((1.0 - gap / 10.0).max(0.0))
}

fn normalized(value: &str) -> String {
    let value = value.trim();
    if value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        crate::smart::fold(value)
    }
}

fn add_people(people: &mut Vec<String>, mut value: String) {
    if value.is_empty() || matches!(value.as_str(), "various artists" | "various" | "unknown artist" | "n/a") {
        return;
    }
    // Most credits name one artist. Move the normalized name into the result
    // directly, without scanning it separately for every collaboration marker.
    if !value.contains([',', ';']) && !value.split(' ').any(|word| {
        matches!(word, "feat." | "feat" | "ft." | "ft" | "featuring" | "vs." | "vs" | "x" | "&" | "and" | "/")
    }) {
        people.push(value);
        return;
    }
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " vs. ", " vs ", " x ", " & ", " and ", " / "] {
        if value.contains(sep) {
            value = value.replace(sep, ",");
        }
    }
    people.extend(value.split([',', ';']).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned));
}

fn credits(t: &Track) -> Vec<String> {
    let mut people = Vec::new();
    for name in [&t.artist, &t.original_artist, &t.remixer] {
        add_people(&mut people, normalized(name));
    }
    // Credits in a title must follow an explicit featured-artist marker. Names in lyrics,
    // ordinary title words, and substrings such as "Air" in "Airport" are not credits.
    // Unicode folding can reveal a featured-artist marker. Plain ASCII titles
    // without an F cannot contain any marker, so avoid allocating a folded copy.
    let bytes = t.title.as_bytes();
    if !bytes.is_ascii() || bytes.contains(&b'f') || bytes.contains(&b'F') {
        let title = normalized(&t.title);
        if title.contains("feat") || title.contains("ft") {
            let title = format!(" {}", title.replace(['(', '['], " "));
            if let Some((start, len)) = [" feat. ", " feat ", " ft. ", " ft ", " featuring "]
                .iter()
                .filter_map(|marker| title.find(marker).map(|i| (i, marker.len())))
                .min()
            {
                let credit = title[start + len..].split([')', ']', '(']).next().unwrap_or("");
                add_people(&mut people, credit.split(" - ").next().unwrap_or("").trim().to_owned());
            }
        }
    }
    people.sort_unstable();
    people.dedup();
    people
}

fn genres<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<_> =
        values.into_iter().flat_map(|s| s.split([';', '/', '|', '\0'])).map(normalized).filter(|s| !s.is_empty()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|v| right.binary_search(v).is_ok())
}

struct Metadata {
    people: Vec<String>,
    label: String,
    album: String,
    genres: Vec<String>,
    provider_genres: Vec<String>,
    styles: Vec<String>,
    provider_labels: Vec<String>,
}

impl Metadata {
    fn new(t: &Track, tags: &[MatchTags]) -> Self {
        let mut provider_labels: Vec<_> = tags.iter().map(|r| normalized(&r.label)).filter(|s| !s.is_empty()).collect();
        provider_labels.sort_unstable();
        provider_labels.dedup();
        Self {
            people: credits(t),
            label: normalized(&t.label),
            album: normalized(&t.album),
            genres: genres([t.genre.as_str()]),
            provider_genres: genres(tags.iter().flat_map(|r| r.genres.iter().map(String::as_str))),
            styles: genres(tags.iter().flat_map(|r| r.styles.iter().map(String::as_str))),
            provider_labels,
        }
    }

    // Local labels take precedence, even when a service suggests a different label.
    fn labels(&self) -> &[String] {
        if self.label.is_empty() { &self.provider_labels } else { std::slice::from_ref(&self.label) }
    }

    fn has_genre(&self, genre: &String) -> bool {
        self.genres.binary_search(genre).is_ok() || self.provider_genres.binary_search(genre).is_ok()
    }

    fn style_matches(&self, style: &String, peer: &Metadata) -> bool {
        let peer_genre = peer.has_genre(style);
        // A style repeated as a genre on both tracks already counts as a genre.
        !(peer_genre && self.has_genre(style)) && (peer_genre || peer.styles.binary_search(style).is_ok())
    }
}

#[derive(Default)]
struct Association {
    comparable: bool,
    credits: usize,
    label: bool,
    album: bool,
    genre: bool,
    provider_label: bool,
    provider_genre: bool,
    provider_style: bool,
}

impl Association {
    fn compare(a: &Metadata, b: &Metadata) -> Self {
        let same = |a: &str, b: &str| !a.is_empty() && a == b;
        let label = same(&a.label, &b.label);
        let provider_label = (a.label.is_empty() || b.label.is_empty())
            && overlap(a.labels(), b.labels());
        let genre = overlap(&a.genres, &b.genres);
        // A provider genre and a local genre may describe the same evidence; count it once.
        let provider_genre = !genre
            && (overlap(&a.provider_genres, &b.genres)
                || overlap(&a.genres, &b.provider_genres)
                || overlap(&a.provider_genres, &b.provider_genres));
        let provider_style = a.styles.iter().any(|s| a.style_matches(s, b)) || b.styles.iter().any(|s| b.style_matches(s, a));
        Self {
            comparable: (!a.people.is_empty() && !b.people.is_empty())
                || (!a.album.is_empty() && !b.album.is_empty())
                || (!a.labels().is_empty() && !b.labels().is_empty())
                || ((!a.genres.is_empty() || !a.provider_genres.is_empty() || !a.styles.is_empty())
                    && (!b.genres.is_empty() || !b.provider_genres.is_empty() || !b.styles.is_empty())),
            credits: a.people.iter().filter(|p| b.people.binary_search(p).is_ok()).count(),
            label,
            provider_label,
            album: same(&a.album, &b.album),
            genre,
            provider_genre,
            provider_style,
        }
    }

    // Build provenance only for a hovered candidate. Ranking the collection needs
    // the same votes, but does not need to allocate provider names for every row.
    fn provider_reasons(&self, a: &Metadata, b: &Metadata, left: &[MatchTags], right: &[MatchTags]) -> Vec<String> {
        let mut origins: [Vec<String>; 3] = Default::default();
        for (owner, peer, tags) in [(a, b, left), (b, a, right)] {
            for row in tags {
                let provider = if row.provider.is_empty() { "OneTagger" } else { &row.provider };
                let label = normalized(&row.label);
                if self.provider_label && owner.label.is_empty() && peer.labels().binary_search(&label).is_ok() {
                    let field = if row.release_id.is_empty() { "label" } else { "release label" };
                    origins[0].push(format!("{provider} {field}"));
                }
                let fields = genres(row.genres.iter().map(String::as_str));
                // An existing local field on this track already explains its side of the match.
                let mut additional = fields.iter().filter(|g| owner.genres.binary_search(g).is_err());
                let genre_source = || {
                    let field = match row.genre_scope.as_str() {
                        "release" => "release genre",
                        "track" => "song genre",
                        "recording" => "recording genre",
                        _ if row.provider == "MusicBrainz" => "recording genre", // earlier cached MusicBrainz rows
                        _ => "genre",
                    };
                    format!("{provider} {field}")
                };
                if self.provider_genre && additional.clone().any(|g| peer.has_genre(g)) {
                    origins[1].push(genre_source());
                }
                if self.provider_style {
                    let styles = genres(row.styles.iter().map(String::as_str));
                    if styles.iter().any(|s| owner.style_matches(s, peer)) {
                        origins[2].push(format!("{provider} style"));
                    }
                    if additional.any(|g| peer.styles.binary_search(g).is_ok() && peer.style_matches(g, owner)) {
                        origins[2].push(genre_source());
                    }
                }
            }
        }
        let mut reasons = Vec::new();
        for (field, mut sources) in ["label", "genre", "style"].into_iter().zip(origins) {
            if !sources.is_empty() {
                sources.sort_unstable();
                sources.dedup();
                reasons.push(format!("Shared {field} ({})", sources.join(", ")));
            }
        }
        reasons
    }

    fn score(&self) -> Option<f64> {
        if !self.comparable {
            return None;
        }
        let mut s: f64 = 0.0;
        if self.credits > 0 {
            s += 0.5 + (self.credits.min(3) - 1) as f64 * 0.1;
        }
        if self.label || self.provider_label {
            s += 0.3;
        }
        if self.album {
            s += 0.15;
        }
        if self.genre || self.provider_genre {
            s += 0.1;
        }
        if self.provider_style {
            s += 0.15;
        }
        Some(s.min(1.0))
    }
}

pub(super) fn mood_features(preview: &[u8]) -> Option<[f64; 4]> {
    let count = preview.len() / 8;
    if count < 16 {
        return None;
    }
    let mut sums = [0u64; 4];
    let mut squares = 0u64;
    for column in preview.chunks_exact(8) {
        for (sum, &v) in sums.iter_mut().zip(&column[4..]) {
            *sum += u64::from(v);
        }
        squares += u64::from(column[4]).pow(2);
    }
    let n = count as f64;
    let [full, low, mid, high] = sums.map(|s| s as f64 / n);
    let bands = low + mid + high;
    if full <= 0.5 || bands <= 0.5 {
        return None;
    }
    let variance = (squares as f64 / n - full * full).max(0.0);
    Some([low / bands, mid / bands, high / bands, (variance.sqrt() / full).min(1.0)])
}

fn sound_similarity(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<f64> {
    let (x, y) = (a?, b?);
    let bands = ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt();
    Some((1.0 - bands / 0.25 - (x[3] - y[3]).abs() / 0.5).max(0.0))
}

fn shared_tags(a: Option<&HashSet<i64>>, b: Option<&HashSet<i64>>) -> Option<(usize, f64)> {
    let (a, b) = (a?, b?);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let count = a.intersection(b).count();
    Some((count, 2.0 * count as f64 / (a.len() + b.len()) as f64))
}

/// Same range as the DJ's BPM filter, relative to the source tempo. Prefer the natural
/// tempo when the three interpretations tie. The multiplier applies to the candidate.
fn tempo(a: f64, b: f64) -> Option<(f64, f64)> {
    if !a.is_finite() || !b.is_finite() || a <= 0.0 || b <= 0.0 {
        return None;
    }
    [1.0, 2.0, 0.5]
        .into_iter()
        .filter_map(|factor| {
            let diff = ((b * factor - a) / a * 100.0).abs();
            diff.is_finite().then_some((diff, factor))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

#[derive(Default)]
struct Weighted {
    total: f64,
    weight: f64,
}
impl Weighted {
    fn add(&mut self, score: Option<f64>, weight: f64) {
        if let Some(s) = score {
            self.total += s * weight;
            self.weight += weight;
        }
    }
    /// A sparse record needs enough independent evidence before it can earn full rank.
    fn score(&self, min_weight: f64) -> Option<f64> {
        (self.weight > 0.0).then(|| (self.total / self.weight.max(min_weight)).min(1.0))
    }
}

struct Signals {
    tempo: Option<(f64, f64)>,
    key: Option<i32>,
    same_mode: Option<bool>,
    era: Option<f64>,
    sound: Option<f64>,
    tags: Option<(usize, f64)>,
}

impl Signals {
    fn new(
        a: &Track,
        b: &Track,
        fa: Option<[f64; 4]>,
        fb: Option<[f64; 4]>,
        ta: Option<&HashSet<i64>>,
        tb: Option<&HashSet<i64>>,
    ) -> Self {
        let key = key_distance(a.key, b.key);
        Self {
            tempo: tempo(a.bpm, b.bpm),
            key,
            same_mode: key.map(|_| a.key / 12 == b.key / 12),
            era: era_score(a, b),
            sound: sound_similarity(fa, fb),
            tags: shared_tags(ta, tb),
        }
    }
    fn tempo_score(&self, limit: f64) -> Option<f64> {
        self.tempo.map(|(d, _)| (1.0 - d / limit).max(0.0))
    }
    fn key_score(&self) -> Option<f64> {
        self.key.map(|d| (1.0 - f64::from(d) / 3.0).max(0.0))
    }
    fn mood(&self) -> Option<f64> {
        let mut m = Weighted::default();
        m.add(self.same_mode.map(|v| if v { 1.0 } else { 0.0 }), 0.25);
        m.add(self.sound, 0.45);
        m.add(self.tempo_score(12.0), 0.15);
        m.add(self.tags.map(|t| t.1), 0.15);
        m.score(0.55)
    }
    fn rank(&self, assoc: &Association) -> f64 {
        // Tempo/key contribute only once. Missing waveform data or provider metadata
        // never counts as a mismatch. Independent evidence supports a stronger rank.
        let mut r = Weighted::default();
        r.add(self.tempo_score(8.0), 0.28);
        r.add(self.key_score(), 0.25);
        r.add(assoc.score(), 0.20);
        r.add(self.era, 0.08);
        r.add(self.sound, 0.12);
        r.add(self.tags.map(|t| t.1), 0.07);
        r.score(0.68).unwrap_or(0.0)
    }
}

fn eligible(rule: &RelatedRules, a: &Track, b: &Track, registered: bool) -> bool {
    let same = |s: &str, t: &str| !s.trim().is_empty() && s.trim().eq_ignore_ascii_case(t.trim());
    (!rule.matching || registered)
        && (rule.key == 0 || key_distance(a.key, b.key).is_some_and(|d| d < i32::from(rule.key)))
        && (!rule.bpm || tempo(a.bpm, b.bpm).is_some_and(|(d, _)| d <= rule.bpm_pct))
        && (!rule.genre || same(&a.genre, &b.genre))
        && (!rule.artist || same(&a.artist, &b.artist))
        && (!rule.label || same(&a.label, &b.label))
        && (!rule.color || (a.color != 0 && a.color == b.color))
        && (!rule.rating || b.rating >= a.rating)
}

struct RankedTrack {
    id: i64,
    score: f64,
    registered: bool,
}

fn order_rows(mut rows: Vec<RankedTrack>) -> Vec<(i64, f64)> {
    rows.sort_unstable_by(|a, b| {
        b.registered.cmp(&a.registered).then_with(|| b.score.total_cmp(&a.score)).then_with(|| a.id.cmp(&b.id))
    });
    rows.into_iter().map(|r| (r.id, r.score)).collect()
}

impl App {
    fn match_sound(&self, id: i64) -> Option<[f64; 4]> {
        let preview = self.previews.get(&id)?;
        *self.match_audio.borrow_mut().entry(id).or_insert_with(|| mood_features(preview))
    }

    fn match_rows(&self, src: i64, target: i64, kind: u8, rules: Option<&RelatedRules>) -> Vec<(i64, f64)> {
        let Some(a) = self.track(src) else { return Vec::new() };
        let metadata = matches!(kind, 0 | 3).then(|| Metadata::new(a, self.match_tags(src)));
        let with_audio = matches!(kind, 0 | 2);
        let sound = if with_audio { self.match_sound(src) } else { None };
        let tags = self.track_tags.get(&src);
        let good: HashSet<_> = self.ext.matches.get(&src).into_iter().flatten().copied().collect();
        let mut rows = Vec::new();
        let mut score_track = |b: &Track| {
            if b.id == src || b.missing {
                return;
            }
            let registered = good.contains(&b.id);
            if rules.is_some_and(|r| !eligible(r, a, b, registered)) {
                return;
            }
            let assoc = || Association::compare(metadata.as_ref().unwrap(), &Metadata::new(b, self.match_tags(b.id)));
            let score = match kind {
                1 => era_score(a, b).unwrap_or(0.0),
                3 => assoc().score().unwrap_or(0.0),
                _ => {
                    let candidate_sound = sound.and_then(|_| self.match_sound(b.id));
                    let signals = Signals::new(a, b, sound, candidate_sound, tags, self.track_tags.get(&b.id));
                    if kind == 2 {
                        signals.mood().unwrap_or(0.0)
                    } else {
                        signals.rank(&assoc())
                    }
                }
            };
            let registered = registered && kind == 0;
            if score > 0.0 || registered || rules.is_some() {
                rows.push(RankedTrack { id: b.id, score, registered });
            }
        };
        if target != 0 && self.node(target).is_some() {
            // A playlist or folder should only incur work for its own candidates.
            let mut seen = HashSet::new();
            for id in self.target_tracks(target) {
                if seen.insert(id) {
                    if let Some(t) = self.track(id) {
                        score_track(t);
                    }
                }
            }
        } else {
            for t in &self.tracks {
                score_track(t);
            }
        }
        order_rows(rows)
    }

    pub fn suggestions(&self, src: i64, target: i64, kind: u8) -> Vec<(i64, f64)> {
        self.match_rows(src, target, kind.min(3), None)
    }

    pub fn related_for(&self, src: i64, target: i64) -> Vec<i64> {
        self.match_rows(src, target, 0, Some(&self.prefs.related)).into_iter().map(|r| r.0).collect()
    }

    pub fn match_explanation(&self, src: i64, candidate: i64, kind: Option<u8>) -> String {
        let (Some(a), Some(b)) = (self.track(src), self.track(candidate)) else { return String::new() };
        let (left, right) = (self.match_tags(src), self.match_tags(candidate));
        let (a_meta, b_meta) = (Metadata::new(a, left), Metadata::new(b, right));
        let assoc = Association::compare(&a_meta, &b_meta);
        let k = kind.unwrap_or(0);
        let sound = if matches!(k, 0 | 2) { self.match_sound(src) } else { None };
        let signals = Signals::new(
            a,
            b,
            sound,
            sound.and_then(|_| self.match_sound(candidate)),
            self.track_tags.get(&src),
            self.track_tags.get(&candidate),
        );
        let mut reasons = Vec::new();
        if k == 0 && self.is_match(src, candidate) {
            reasons.push("Registered matching pair".to_owned());
        }
        if matches!(k, 0 | 2) {
            if let Some((diff, factor)) = signals.tempo.filter(|(d, _)| *d < if k == 2 { 12.0 } else { 8.0 }) {
                let ratio = if factor == 2.0 {
                    " (candidate at half tempo)"
                } else if factor == 0.5 {
                    " (candidate at double tempo)"
                } else {
                    ""
                };
                reasons.push(format!("Tempo difference {diff:.1}%{ratio}"));
            }
            if k == 0 {
                match signals.key {
                    Some(0) => reasons.push("Same key".into()),
                    Some(1) => reasons.push("Neighbor on the key wheel".into()),
                    Some(2) => reasons.push("Two steps on the key wheel".into()),
                    _ => {}
                }
            } else if signals.same_mode == Some(true) {
                reasons.push("Same major/minor mode".into());
            }
            if signals.sound.is_some_and(|s| s > 0.5) {
                reasons.push("Similar spectral balance and dynamics".into());
            }
            if let Some((n, _)) = signals.tags.filter(|(n, _)| *n > 0) {
                reasons.push(format!("Shared My Tags: {n}"));
            }
        }
        if matches!(k, 0 | 1) && signals.era.is_some_and(|v| v > 0.0) {
            reasons.push(format!("Release years {} and {}", year_of(a).unwrap(), year_of(b).unwrap()));
        }
        if matches!(k, 0 | 3) {
            if assoc.credits > 0 {
                reasons.push("Shared artist or remix credit".into());
            }
            if assoc.label {
                reasons.push("Same library label".into());
            }
            if assoc.album {
                reasons.push("Same album".into());
            }
            if assoc.genre {
                reasons.push("Shared library genre".into());
            }
            if assoc.provider_label || assoc.provider_genre || assoc.provider_style {
                reasons.extend(assoc.provider_reasons(&a_meta, &b_meta, left, right));
            }
        }
        if reasons.is_empty() && kind.is_none() {
            reasons.push("Meets the selected Related Tracks criteria".into());
        }
        reasons.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tr(id: i64, bpm: f64, key: i32, year: i32, artist: &str, label: &str, genre: &str) -> Track {
        Track { id, bpm, key, year, artist: artist.into(), label: label.into(), genre: genre.into(), ..Default::default() }
    }

    #[test]
    fn era_association_and_mix() {
        let a = tr(1, 128.0, 0, 2020, "Max Styler", "Nervous", "Tech House");
        let same_year = tr(2, 128.0, 0, 2020, "X", "", "");
        let five = tr(3, 128.0, 0, 2015, "X", "", "");
        let far = tr(4, 128.0, 0, 1995, "X", "", "");
        let unknown = tr(5, 128.0, 0, 0, "X", "", "");
        assert_eq!(era_score(&a, &same_year), Some(1.0));
        assert_eq!(era_score(&a, &five), Some(0.5));
        assert_eq!(era_score(&a, &far), Some(0.0));
        assert_eq!(era_score(&a, &unknown), None);
        // Shared artist beats shared label beats nothing; "feat." names count.
        let art = tr(6, 100.0, 5, 0, "Dale Howard, Max Styler", "", "");
        let lab = tr(7, 100.0, 5, 0, "Someone", "nervous", "");
        let none = tr(8, 100.0, 5, 0, "Someone", "Other", "House");
        let mut feat = tr(9, 100.0, 5, 0, "Someone", "", "");
        feat.title = "Night (feat. Max Styler)".into();
        let association_score =
            |a, b| Association::compare(&Metadata::new(a, &[]), &Metadata::new(b, &[])).score().unwrap_or(0.0);
        assert!(association_score(&a, &art) > association_score(&a, &lab));
        assert!(association_score(&a, &lab) > 0.0);
        assert_eq!(association_score(&a, &none), 0.0);
        assert!(association_score(&a, &feat) > 0.0);
        // Mixing: same tempo and key is best; half tempo counts as the same tempo.
        let half = tr(10, 64.0, 0, 0, "", "", "");
        let off = tr(11, 140.0, 6, 0, "", "", "");
        let m = |t| Signals::new(&a, t, None, None, None, None);
        assert_eq!(m(&same_year).tempo_score(8.0), Some(1.0));
        assert_eq!(m(&same_year).key_score(), Some(1.0));
        assert_eq!(m(&half).tempo, Some((0.0, 2.0)));
        assert_eq!(m(&off).key_score(), Some(0.0));
        assert_eq!(m(&tr(12, 0.0, -1, 0, "", "", "")).tempo, None);
    }

    #[test]
    fn mood_and_ranking() {
        let col = |low: u8, mid: u8, high: u8| -> Vec<u8> {
            (0..400).flat_map(|_| [200, low, mid, high, 100, low, mid, high]).collect()
        };
        let bassy = mood_features(&col(120, 40, 10)).unwrap();
        let bassy2 = mood_features(&col(115, 42, 12)).unwrap();
        let bright = mood_features(&col(20, 60, 110)).unwrap();
        assert!(mood_features(&[]).is_none());
        let a = tr(1, 128.0, 12, 0, "", "", "");
        let b = tr(2, 128.0, 14, 0, "", "", "");
        let close = Signals::new(&a, &b, Some(bassy), Some(bassy2), None, None).mood().unwrap();
        let apart = Signals::new(&a, &b, Some(bassy), Some(bright), None, None).mood().unwrap();
        assert!(close > 0.9 && apart < close - 0.3, "{close} {apart}");
        // Ranking: best first; Era leaves out tracks that score nothing; a registered match
        // always leads the Rank list.
        let src = tr(1, 128.0, 0, 2020, "A", "L", "House");
        let good = tr(2, 128.0, 0, 2020, "A", "L", "House");
        let ok = tr(3, 126.0, 7, 2012, "B", "", "House");
        let poor = tr(4, 90.0, 6, 1990, "C", "", "Rock");
        let scores = |kind: u8, registered: i64| {
            let mut rows = vec![];
            for t in [&poor, &ok, &good] {
                let sig = Signals::new(&src, t, None, None, None, None);
                let assoc = Association::compare(&Metadata::new(&src, &[]), &Metadata::new(t, &[]));
                let score = match kind {
                    1 => sig.era.unwrap_or(0.0),
                    3 => assoc.score().unwrap_or(0.0),
                    _ => sig.rank(&assoc),
                };
                let registered = t.id == registered;
                if score > 0.0 || registered {
                    rows.push(RankedTrack { id: t.id, score, registered });
                }
            }
            order_rows(rows)
        };
        let rank = scores(0, 0);
        assert_eq!(rank.iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
        assert!(rank[0].1 > 0.95);
        assert_eq!(scores(1, 0).iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(scores(3, 0).iter().map(|r| r.0).collect::<Vec<_>>(), vec![2, 3]);
        let matched = scores(0, 4);
        assert_eq!(matched[0], (4, 0.0));
    }

    #[test]
    fn sparse_evidence_cannot_equal_a_well_documented_match() {
        let song = tr(1, 126.0, 5, 2020, "Basement", "Sands", "House");
        let sparse = tr(2, 126.0, -1, 0, "", "", "");
        let confirmed = tr(3, 124.0, 6, 2021, "Basement", "Sands", "House");
        let score = |other| {
            let meta = Association::compare(&Metadata::new(&song, &[]), &Metadata::new(other, &[]));
            Signals::new(&song, other, None, None, None, None).rank(&meta)
        };
        assert!(score(&sparse) > 0.0 && score(&sparse) < 0.5);
        assert!(score(&confirmed) > score(&sparse));
        assert_eq!(score(&tr(5, f64::NAN, -1, 0, "", "", "")), 0.0);
        let tied =
            |order: &[i64]| order_rows(order.iter().map(|&id| RankedTrack { id, score: 0.5, registered: false }).collect());
        assert_eq!(tied(&[3, 2]), tied(&[2, 3]));
    }

    #[test]
    fn substring_or_compilation_artist_is_not_a_recording_credit() {
        let artist = tr(1, 0.0, -1, 0, "Air", "", "");
        let mut other = tr(2, 0.0, -1, 0, "Different Artist", "", "");
        fn shared(left: &Track, right: &Track) -> usize {
            Association::compare(&Metadata::new(left, &[]), &Metadata::new(right, &[])).credits
        }
        for title in ["Airport Nights", "Love Is In The Air", "Metro (feat. Airwaves)"] {
            other.title = title.into();
            assert_eq!(shared(&artist, &other), 0);
        }
        other.title = "Sunrise [feat. AIR]".into();
        assert_eq!(shared(&artist, &other), 1);
        let mut a = artist.clone();
        a.artist = "Various Artists".into();
        other.title.clear();
        other.artist = "Various Artists".into();
        assert_eq!(shared(&a, &other), 0);
    }

    #[test]
    fn credits_keep_multiword_names_and_fold_featured_artist_markers() {
        let mut track = Track { artist: "  The Blue Note  ".into(), ..Default::default() };
        assert_eq!(credits(&track), ["the blue note"]);
        track.artist = "Air feat. One; Two / Three vs. Four".into();
        track.title = "Étude [ＦＥＡＴ. Mañana]".into();
        assert_eq!(credits(&track), ["air", "four", "manana", "one", "three", "two"]);
        track.artist = "Daft Punk".into();
        track.title = "Mixtape after midnight".into();
        assert_eq!(credits(&track), ["daft punk"]);
    }

    #[test]
    fn provider_votes_keep_local_precedence_and_identify_only_contributing_evidence() {
        let left = tr(1, 120.0, -1, 0, "", "Pampa", "House");
        let mut right = tr(2, 120.0, -1, 0, "", "", "");
        let apple = MatchTags { provider: "iTunes".into(), genre_scope: "track".into(), genres: vec!["Techno".into()], ..Default::default() };
        let recording = MatchTags { provider: "MusicBrainz".into(), genres: vec!["House".into()], ..Default::default() };
        let release = MatchTags {
            provider: "Deezer".into(), genre_scope: "release".into(), release_id: "123".into(),
            genres: vec!["House".into()], label: "Pampa".into(), ..Default::default()
        };
        let first = [apple.clone()];
        let second = [release.clone(), recording.clone(), apple.clone()];
        // Apple contributes a different, corroborated genre; all corroborated genres still earn one overall vote.
        let a = Metadata::new(&left, &first);
        let b = Metadata::new(&right, &second);
        let assoc = Association::compare(&a, &b);
        assert_eq!(assoc.score(), Association::compare(&Metadata::new(&left, &[]), &Metadata::new(&right, std::slice::from_ref(&release))).score());
        assert_eq!(assoc.provider_reasons(&a, &b, &first, &second), [
            "Shared label (Deezer release label)",
            "Shared genre (Deezer release genre, MusicBrainz recording genre, iTunes song genre)",
        ]);

        // Right-side label conflict is authoritative, the unrelated Apple genre on just one side no longer contributes.
        right.label = "Another label".into();
        let a = Metadata::new(&left, &[]);
        let b = Metadata::new(&right, &second);
        let assoc = Association::compare(&a, &b);
        assert!(!assoc.provider_label);
        assert_eq!(assoc.provider_reasons(&a, &b, &[], &second), [
            "Shared genre (Deezer release genre, MusicBrainz recording genre)",
        ]);
        // A local genre present on both sides needs no provider attribution, even when several services agree.
        right.genre = "House".into();
        let b = Metadata::new(&right, &second);
        let assoc = Association::compare(&a, &b);
        assert!(assoc.genre && !assoc.provider_genre);
        assert!(assoc.provider_reasons(&a, &b, &[], &second).is_empty());
    }

    #[test]
    fn a_style_match_credits_the_genre_source_on_the_other_track() {
        let empty = Track::default();
        let left = [MatchTags {
            provider: "Style fixture".into(), styles: vec!["Garage".into()], ..Default::default()
        }];
        let right = [MatchTags {
            provider: "Deezer".into(), genres: vec!["Garage".into()], genre_scope: "release".into(),
            release_id: "321".into(), ..Default::default()
        }];
        let a = Metadata::new(&empty, &left);
        let b = Metadata::new(&empty, &right);
        let assoc = Association::compare(&a, &b);
        assert!(assoc.provider_style && !assoc.provider_genre);
        assert_eq!(assoc.provider_reasons(&a, &b, &left, &right), [
            "Shared style (Deezer release genre, Style fixture style)",
        ]);
    }

    #[test]
    fn enrichment_adds_corroborated_evidence_and_keeps_strict_filters_local() {
        let a = tr(1, 128.0, 0, 0, "A", "", "House");
        let b = tr(2, 64.0, 0, 0, "B", "Pampa", "Deep House; Dyñamic");
        let tags = MatchTags {
            genres: vec!["DYNAMIC".into()],
            styles: vec!["Deep House".into()],
            label: "Pampa".into(),
            ..Default::default()
        };
        let local = Association::compare(&Metadata::new(&a, &[]), &Metadata::new(&b, &[]));
        let enriched = Association::compare(&Metadata::new(&a, std::slice::from_ref(&tags)), &Metadata::new(&b, &[]));
        assert!(enriched.provider_genre && enriched.provider_style && enriched.provider_label);
        assert!(enriched.score().unwrap() > local.score().unwrap_or(0.0));
        assert!(eligible(&RelatedRules::default(), &a, &b, false));
        for requirements in [
            RelatedRules { genre: true, ..Default::default() },
            RelatedRules { label: true, ..Default::default() },
            RelatedRules { matching: true, ..Default::default() },
        ] {
            assert!(!eligible(&requirements, &a, &b, false));
        }
        let mut invalid = b.clone();
        for bpm in [0.0, f64::NAN, f64::INFINITY] {
            invalid.bpm = bpm;
            assert!(!eligible(&RelatedRules::default(), &a, &invalid, true));
        }
        let same_genre = Metadata::new(
            &a,
            &[MatchTags { genres: vec!["House".into()], styles: vec!["House".into()], ..Default::default() }],
        );
        let assoc = Association::compare(&same_genre, &same_genre);
        assert!(assoc.genre);
        assert!(!assoc.provider_genre && !assoc.provider_style);
        let provider_only = Metadata::new(
            &Track::default(),
            &[MatchTags { genres: vec!["house".into()], styles: vec!["House".into()], ..Default::default() }],
        );
        let mixed = Association::compare(&provider_only, &Metadata::new(&a, &[]));
        assert!(mixed.provider_genre && !mixed.provider_style);
    }
}
