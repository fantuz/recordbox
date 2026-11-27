//! Separate the MIDI service's current connection identity from the name used by
//! recordbox and AlphaTheta for mapping files. Never guess between competing inputs.
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct Inputs {
    unique: HashMap<String, String>,
    pub(super) paused: Vec<String>,
}

impl Inputs {
    pub(super) fn inspect<'a>(ports: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let mut names = HashMap::<_, Vec<_>>::new();
        for (name, id) in ports {
            let alsa = crate::midi_csv::alsa_key(name);
            let key = (
                alsa.is_some(),
                alsa.as_deref().unwrap_or(name).to_ascii_lowercase(),
            );
            names.entry(key).or_default().push((name, id));
        }
        let mut unique = HashMap::new();
        let mut paused = BTreeSet::new();
        for group in names.into_values() {
            if let [(name, id)] = group.as_slice() {
                unique.insert((*name).to_owned(), (*id).to_owned());
            } else {
                paused.extend(group.into_iter().map(|(name, _)| name.to_owned()));
            }
        }
        Self {
            unique,
            paused: paused.into_iter().collect(),
        }
    }

    pub(super) fn contains(&self, name: &str, id: &str) -> bool {
        self.unique.get(name).is_some_and(|current| current == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_os_endpoint_replaces_a_same_named_connection() {
        let inputs = Inputs::inspect([("DDJ", "interface-new")].into_iter());
        assert!(!inputs.contains("DDJ", "interface-old"));
        assert!(inputs.contains("DDJ", "interface-new"));
        assert!(inputs.paused.is_empty());
    }

    #[test]
    fn equal_labels_pause_all_contenders_until_the_ambiguity_clears() {
        for ids in [["one", "two"], ["two", "one"], ["same", "same"]] {
            let inputs =
                Inputs::inspect([("DDJ", ids[0]), ("DDJ", ids[1]), ("DDJ 2", "three")].into_iter());
            assert_eq!(inputs.paused, ["DDJ"]);
            assert!(!inputs.contains("DDJ", ids[0]));
            assert!(!inputs.contains("DDJ", ids[1]));
            assert!(inputs.contains("DDJ 2", "three"));
        }
        assert!(Inputs::inspect([("DDJ", "two")].into_iter()).contains("DDJ", "two"));
    }

    #[test]
    fn distinct_numbered_ports_remain_available_even_on_one_interface() {
        let inputs = Inputs::inspect(
            [
                ("MIDIIN (DDJ)", "interface"),
                ("MIDIIN2 (DDJ)", "interface"),
            ]
            .into_iter(),
        );
        assert!(inputs.contains("MIDIIN (DDJ)", "interface"));
        assert!(inputs.contains("MIDIIN2 (DDJ)", "interface"));
        assert!(inputs.paused.is_empty());
    }

    #[test]
    fn two_alsa_clients_with_the_same_persistent_key_cannot_steal_each_others_map() {
        let a = "Deck:Deck MIDI 1 24:0";
        let b = "Deck:Deck MIDI 1 35:0";
        let independent = "Deck:Deck MIDI 2 35:1";
        let inputs = Inputs::inspect([(a, "24:0"), (b, "35:0"), (independent, "35:1")].into_iter());
        assert_eq!(inputs.paused, [a, b]);
        assert!(!inputs.contains(a, "24:0"));
        assert!(!inputs.contains(b, "35:0"));
        assert!(inputs.contains(independent, "35:1"));
    }
}
