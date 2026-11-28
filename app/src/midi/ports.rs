//! Separate the MIDI service's current connection identity from the name used by
//! recordbox and AlphaTheta for mapping files. Never guess between competing inputs.
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Weak};

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

/// An input and its feedback destination are separate OS endpoints, even when their
/// display names happen to agree. A returned output must have one possible sender.
pub(super) fn paired_outputs(inputs: &[String], outputs: &[String]) -> Vec<Option<usize>> {
    let paired: Vec<_> = inputs.iter().map(|name| crate::midi_csv::paired_output(name, outputs)).collect();
    let mut claims = vec![0usize; outputs.len()];
    for &index in paired.iter().flatten() { claims[index] += 1; }
    paired.into_iter().map(|index| index.filter(|&i| claims[i] == 1)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub(super) name: String,
    pub(super) id: String,
}

/// The worker keeps its claim until its OS connection has closed. A replacement input
/// can start immediately, even when the old output worker is still inside a driver send.
#[derive(Default)]
pub(super) struct OutputClaims(HashMap<String, Weak<()>>);

impl OutputClaims {
    pub(super) fn acquire(&mut self, id: &str) -> Option<Arc<()>> {
        self.0.retain(|_, owner| owner.strong_count() > 0);
        if self.0.contains_key(id) {
            return None;
        }
        let lease = Arc::new(());
        self.0.insert(id.to_owned(), Arc::downgrade(&lease));
        Some(lease)
    }
}

/// An old worker closes its own OS handle before a scanner may open a replacement.
#[derive(Default)]
pub(super) enum FeedbackLink {
    #[default]
    Idle,
    Open(Endpoint),
    Closing,
}

impl FeedbackLink {
    pub(super) fn retire_if_changed(&mut self, desired: Option<&Endpoint>) -> bool {
        if matches!(self, Self::Open(open) if Some(&*open) != desired) {
            *self = Self::Closing;
            true
        } else {
            false
        }
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
    #[test]
    fn two_input_names_cannot_send_feedback_to_one_output() {
        let inputs = ["Controller Input", "Controller Output", "Other In"].map(str::to_string);
        let outputs = ["Controller Output", "Other Out"].map(str::to_string);
        assert_eq!(paired_outputs(&inputs, &outputs), [None, None, Some(1)]);
        assert_eq!(paired_outputs(&inputs[..1], &outputs), [Some(0)]);
        let duplicate = ["Controller Output", "Controller Output"].map(str::to_string);
        assert_eq!(paired_outputs(&inputs[..1], &duplicate), [None]);
    }

    #[test]
    fn a_replacement_input_waits_until_the_previous_output_handle_is_released() {
        let mut claims = OutputClaims::default();
        let old_worker = claims.acquire("output-1").unwrap();
        assert!(claims.acquire("output-1").is_none());
        let independent = claims.acquire("output-2").unwrap();
        drop(old_worker);
        let _replacement = claims.acquire("output-1").unwrap();
        assert!(claims.acquire("output-2").is_none());
        drop(independent);
    }

    #[test]
    fn output_hotplug_tracks_its_own_endpoint_and_waits_for_worker_shutdown() {
        let key = |id: &str| Endpoint { name: "Controller Out".into(), id: id.into() };
        let mut link = FeedbackLink::Open(key("destination-1"));
        assert!(!link.retire_if_changed(Some(&key("destination-1"))));
        assert!(link.retire_if_changed(Some(&key("destination-2"))));
        assert!(matches!(link, FeedbackLink::Closing));
        assert!(!link.retire_if_changed(None));
        assert!(FeedbackLink::Open(key("destination-2")).retire_if_changed(None));
    }

}
