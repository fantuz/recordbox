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

/// On ALSA the visible label wraps the virtual port in a client and numeric address.
fn reserved(name: &str) -> bool {
    let leaf = crate::midi_csv::alsa_endpoint(name).map_or(name, |(_, leaf, _, _)| leaf);
    leaf.starts_with("rbxtest-")
}

fn scripted_alsa(name: &str) -> Option<&str> {
    let (client, leaf, _, _) = crate::midi_csv::alsa_endpoint(name)?;
    (client == "recordbox-test" && leaf.starts_with("rbxtest-")).then_some(leaf)
}

pub(crate) fn is_test_input(name: &str, expected: &str) -> bool {
    expected.starts_with("rbxtest-") && (name == expected || scripted_alsa(name) == Some(expected))
}

pub(super) fn allowed_input(name: &str, ignore: &[String], only: Option<&[String]>) -> bool {
    !ignore.iter().any(|n| n == name)
        && match only {
            Some(names) => names.iter().any(|n| n == name || is_test_input(name, n)),
            None => !reserved(name),
        }
}

/// An input and its feedback destination are separate OS endpoints, even when their
/// display names happen to agree. A returned output must have one possible sender.
pub(super) fn paired_outputs(inputs: &[String], outputs: &[String]) -> Vec<Option<usize>> {
    let paired: Vec<_> = inputs
        .iter()
        .map(|name| {
            // The scripted UI creates its two virtual endpoints as distinct ALSA clients.
            // Only its reserved namespace may pair across client IDs.
            if let Some(leaf) = scripted_alsa(name) {
                let mut hits = outputs
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| scripted_alsa(n) == Some(leaf));
                let first = hits.next()?.0;
                return hits.next().is_none().then_some(first);
            }
            crate::midi_csv::paired_output(name, outputs)
        })
        .collect();
    let mut claims = vec![0usize; outputs.len()];
    for &index in paired.iter().flatten() {
        claims[index] += 1;
    }
    paired
        .into_iter()
        .map(|index| index.filter(|&i| claims[i] == 1))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub(super) name: String,
    pub(super) id: String,
}

/// The worker keeps its claim until its OS connection has closed. A replacement input
/// can start immediately, even when the old output worker is still inside a driver send.
#[derive(Default)]
pub(super) struct OutputClaims(HashMap<(String, String), Weak<()>>);

impl OutputClaims {
    pub(super) fn acquire(&mut self, endpoint: &Endpoint) -> Option<Arc<()>> {
        self.0.retain(|_, owner| owner.strong_count() > 0);
        // WinMM identifies a numbered endpoint by both interface ID and port name.
        // CoreMIDI and ALSA keep the same endpoint ID across a display-name change.
        let name = if cfg!(windows) {
            endpoint.name.clone()
        } else {
            String::new()
        };
        let key = (endpoint.id.clone(), name);
        if self.0.contains_key(&key) {
            return None;
        }
        let lease = Arc::new(());
        self.0.insert(key, Arc::downgrade(&lease));
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
    fn reserved_virtual_ports_keep_their_alsa_identity_and_cannot_claim_a_real_output() {
        let name = "rbxtest-midi4 controller";
        let own = format!("recordbox-test:{name} 34:0");
        let out = format!("recordbox-test:{name} 35:0");
        let other = format!("Unrelated:{name} 36:0");
        let only = [name.to_owned()];
        assert!(!allowed_input(&own, &[], None));
        assert!(allowed_input(&own, &[], Some(&only)));
        assert!(!allowed_input(&other, &[], Some(&only)));
        assert!(!allowed_input(
            &own,
            std::slice::from_ref(&own),
            Some(&only)
        ));
        assert!(allowed_input(
            "recordbox virtual controller:Rehearsal 42:0",
            &[],
            None
        ));
        let inputs = [own];
        assert_eq!(
            paired_outputs(&inputs, &[out.clone(), other.clone()]),
            [Some(0)]
        );
        assert_eq!(paired_outputs(&inputs, &[other]), [None]);
        assert_eq!(paired_outputs(&inputs, &[out.clone(), out]), [None]);
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
        let first = Endpoint {
            name: "MIDI Out".into(),
            id: "output-1".into(),
        };
        let second = Endpoint {
            name: "MIDI Out 2".into(),
            id: "output-2".into(),
        };
        let old_worker = claims.acquire(&first).unwrap();
        assert!(claims.acquire(&first).is_none());
        let independent = claims.acquire(&second).unwrap();
        drop(old_worker);
        let _replacement = claims.acquire(&first).unwrap();
        assert!(claims.acquire(&second).is_none());
        drop(independent);
    }

    #[test]
    fn output_hotplug_tracks_its_own_endpoint_and_waits_for_worker_shutdown() {
        let key = |id: &str| Endpoint {
            name: "Controller Out".into(),
            id: id.into(),
        };
        let mut link = FeedbackLink::Open(key("destination-1"));
        assert!(!link.retire_if_changed(Some(&key("destination-1"))));
        assert!(link.retire_if_changed(Some(&key("destination-2"))));
        assert!(matches!(link, FeedbackLink::Closing));
        assert!(!link.retire_if_changed(None));
        assert!(FeedbackLink::Open(key("destination-2")).retire_if_changed(None));
    }
}
