//! Deterministic collection fixture. The study runner supplies the actual scoring modules.
#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
mod smart;
mod model {
    #[derive(Clone, Default)]
    pub struct Track {
        pub id: i64,
        pub bpm: f64,
        pub key: i32,
        pub rating: u8,
        pub color: u8,
        pub year: i32,
        pub missing: bool,
        pub path: String,
        pub mix_name: String,
        pub artist: String,
        pub genre: String,
        pub label: String,
        pub title: String,
        pub remixer: String,
        pub album: String,
        pub original_artist: String,
        pub release_date: String,
    }
}
mod onetagger {
    #[derive(Clone, Default)]
    pub struct MatchTags {
        pub genres: Vec<String>,
        pub styles: Vec<String>,
        pub label: String,
        pub provider: String,
        pub genre_scope: String,
        pub release_id: String,
    }
}
mod app {
    use super::*;
    #[derive(Clone)]
    pub struct RelatedRules {
        pub key: u8,
        pub bpm: bool,
        pub bpm_pct: f64,
        pub genre: bool,
        pub artist: bool,
        pub label: bool,
        pub color: bool,
        pub rating: bool,
        pub matching: bool,
    }
    impl Default for RelatedRules {
        fn default() -> Self {
            Self {
                key: 2,
                bpm: true,
                bpm_pct: 6.0,
                genre: false,
                artist: false,
                label: false,
                color: false,
                rating: false,
                matching: false,
            }
        }
    }
    include!("key_distance.rs");
    #[derive(Default)]
    pub struct Prefs {
        pub related: RelatedRules,
    }
    #[derive(Default)]
    pub struct Ext {
        pub matches: HashMap<i64, Vec<i64>>,
    }

    pub struct Identity {
        pub id: i64,
        pub title: String,
        pub artist: String,
        pub path: String,
        pub mix_name: String,
    }
    impl Identity {
        fn agrees(&self, t: &model::Track) -> bool {
            self.id == t.id
                && self.artist == t.artist
                && self.title == t.title
                && self.mix_name == t.mix_name
                && self.path == t.path
        }
    }
    #[derive(Default)]
    pub struct App {
        pub tracks: Vec<model::Track>,
        pub labels: HashMap<i64, (Identity, Vec<onetagger::MatchTags>)>,
        pub index: HashMap<i64, usize>,
        pub previews: HashMap<i64, Vec<u8>>,
        pub match_audio: RefCell<HashMap<i64, Option<[f64; 4]>>>,
        pub track_tags: HashMap<i64, HashSet<i64>>,
        pub ext: Ext,
        pub prefs: Prefs,
    }
    impl App {
        pub fn track(&self, i: i64) -> Option<&model::Track> {
            self.index.get(&i).and_then(|n| self.tracks.get(*n))
        }
        pub fn match_tags(&self, i: i64) -> &[onetagger::MatchTags] {
            self.labels
                .get(&i)
                .filter(|(k, _)| self.track(i).is_some_and(|t| k.agrees(t)))
                .map_or(&[], |(_, v)| v.as_slice())
        }
        pub fn before_match_tags(&self, i: i64) -> Option<&onetagger::MatchTags> {
            self.match_tags(i).first()
        }
        pub fn node(&self, _: i64) -> Option<()> {
            None
        }
        pub fn is_match(&self, a: i64, b: i64) -> bool {
            self.ext.matches.get(&a).is_some_and(|ls| ls.contains(&b))
        }
        pub fn target_tracks(&self, _: i64) -> Vec<i64> {
            self.tracks.iter().map(|t| t.id).collect()
        }
    }
}
mod before;
mod candidate;
mod legacy;
mod submit;
use std::time::Instant;

fn build(n: i64, providers: usize, tagged: bool) -> app::App {
    let mut app = app::App::default();
    for i in 0..n {
        let mut t = model::Track {
            id: i,
            path: format!("/library/{:03}/song {i:05}.flac", i % 122),
            title: format!(
                "{} {}",
                ["Soul", "Ocean", "Blue", "Bésame", "Electric", "Dance"][i as usize % 6],
                i
            ),
            bpm: if i % 37 == 0 {
                0.0
            } else {
                80.0 + ((i * 781) % 901) as f64 / 10.0
            },
            key: if i % 19 == 0 {
                -1
            } else {
                ((i * 5) % 24) as i32
            },
            year: if i % 13 == 0 {
                0
            } else {
                1980 + ((i * 7) % 47) as i32
            },
            artist: if i % 41 == 0 {
                String::new()
            } else {
                format!("Artist {}", i % 801)
            },
            album: if i % 7 == 0 {
                String::new()
            } else {
                format!("Album {}", i % 703)
            },
            label: if i % 5 == 0 {
                String::new()
            } else {
                format!("Label {}", i % 51)
            },
            genre: if i % 4 == 0 {
                String::new()
            } else {
                [
                    "House",
                    "Dance",
                    "Garage / House",
                    "Drum & Bass",
                    "Tech House",
                    "Electrónica",
                    "Disco",
                    "Jazz",
                    "Soul",
                    "Ambient",
                    "Progressive / Dance",
                ][(i % 11) as usize]
                    .to_owned()
            },
            missing: i % 193 == 0,
            ..Default::default()
        };
        if i % 15 == 0 {
            t.artist.push_str(" feat. Petit");
        }
        if i % 13 == 0 {
            t.title.push_str(" (ft. Jade)");
        }
        if i % 6 == 0 {
            t.mix_name = format!("Remix {}", i % 11);
        }
        if i % 25 == 0 {
            t.remixer = format!("Remixer {}", i % 29);
        }
        if !tagged {
            t.title = format!("Track {i:05}");
            t.artist.clear();
            t.album.clear();
            t.genre.clear();
            t.label.clear();
            t.remixer.clear();
            t.mix_name.clear();
            t.year = 0;
        }
        if i % 7 != 0 {
            let spec = [i % 27, i % 43, i % 57];
            let preview: Vec<u8> = (0..400)
                .flat_map(|c| {
                    [
                        127,
                        68,
                        87,
                        35,
                        82 + ((i + c) % 73) as u8,
                        74 + ((spec[0] + c) % 51) as u8,
                        54 + ((spec[1] + 2 * c) % 74) as u8,
                        33 + ((spec[2] + c) % 79) as u8,
                    ]
                })
                .collect();
            app.previews.insert(i, preview);
        }
        if tagged && i % 5 == 0 {
            app.track_tags
                .insert(i, HashSet::from([i % 21, i % 13 + 30]));
        }
        if providers > 0 && i % 10 != 0 {
            let k = app::Identity {
                id: i,
                path: t.path.clone(),
                artist: t.artist.clone(),
                title: t.title.clone(),
                mix_name: t.mix_name.clone(),
            };
            let mut rows = Vec::new();
            for p in 0..providers {
                let broad = [
                    "Dance",
                    "House",
                    "Pop",
                    "Soul",
                    "Electronic",
                    "Jazz",
                    "Rock",
                    "Classical",
                ][(i % 8) as usize];
                let alt = [
                    "Trip hop",
                    "Alternative Dance",
                    "Disco",
                    "Rap",
                    "Trance",
                    "Rock / Pop",
                    "Soul",
                    "Electronica",
                ][(i % 8) as usize];
                rows.push(onetagger::MatchTags {
                    provider: ["MusicBrainz", "iTunes", "Deezer"][p].into(),
                    genres: if p == 1 {
                        vec![broad.into()]
                    } else {
                        vec![broad.into(), alt.into()]
                    },
                    styles: if p == 0 {
                        vec![format!("Style {}", i % 23)]
                    } else {
                        vec![]
                    },
                    label: if p == 2 {
                        format!("Label {}", i % 51)
                    } else {
                        String::new()
                    },
                    ..Default::default()
                });
            }
            app.labels.insert(i, (k, rows));
        }
        app.index.insert(i, app.tracks.len());
        app.tracks.push(t);
    }
    app.ext.matches.insert(31, vec![1002, 501, 14]);
    app
}
fn digest(rows: &[(i64, f64)]) -> u64 {
    rows.iter().fold(17u64, |h, (id, score)| {
        h.wrapping_mul(1000000007).wrapping_add(*id as u64) ^ score.to_bits()
    })
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let count: i64 = args[0].parse().unwrap();
    let providers: usize = args[1].parse().unwrap();
    let repeats: usize = args[2].parse().unwrap();
    let src: i64 = args[3].parse().unwrap();
    assert!(count > src && matches!(providers, 0 | 1 | 3) && repeats > 0);
    let catalog = &args[4];
    let app = build(count, providers, catalog == "tagged");
    for trial in 0..repeats {
        // All versions share the immutable library. Rotate order to distribute
        // thermal drift and reset just the waveform summary cache for a cold rank.
        let mut versions = match providers {
            0 => vec!["upstream", "before", "submitted", "candidate"],
            1 => vec!["before", "submitted", "candidate"],
            _ => vec!["submitted", "candidate"],
        };
        let mut related_ids = None;
        let versions_len = versions.len();
        versions.rotate_left(trial % versions_len);
        for version in versions {
            app.match_audio.borrow_mut().clear();
            for (mode, kind) in [("cold", 0), ("warm", 0), ("association", 3), ("related", 4)] {
                let start = Instant::now();
                let rows = match (version, kind) {
                    ("upstream", 4) => app
                        .legacy_related_tracks_of(src)
                        .into_iter()
                        .map(|id| (id, 0.0))
                        .collect(),
                    ("before", 4) => app
                        .before_related_for(src, 0)
                        .into_iter()
                        .map(|id| (id, 0.0))
                        .collect(),
                    ("submitted", 4) => app
                        .submit_related_for(src, 0)
                        .into_iter()
                        .map(|id| (id, 0.0))
                        .collect(),
                    ("candidate", 4) => app
                        .related_for(src, 0)
                        .into_iter()
                        .map(|id| (id, 0.0))
                        .collect(),
                    ("upstream", k) => app.legacy_suggestions(src, 0, k),
                    ("before", k) => app.before_suggestions(src, 0, k),
                    ("submitted", k) => app.submit_suggestions(src, 0, k),
                    ("candidate", k) => app.suggestions(src, 0, k),
                    _ => unreachable!(),
                };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                if kind == 4 {
                    let mut ids: Vec<_> = rows.iter().map(|row| row.0).collect();
                    ids.sort_unstable();
                    if let Some(reference) = &related_ids {
                        assert_eq!(&ids, reference, "Related Tracks eligible set changed");
                    } else {
                        related_ids = Some(ids);
                    }
                }
                println!(
                    "{count},{catalog},{providers},{src},{trial},{version},{mode},{},{:016x},{elapsed:.3}",
                    rows.len(),
                    digest(&rows)
                );
            }
        }
    }
}
