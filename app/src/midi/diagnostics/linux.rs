//! Publish one duplex ALSA address so recordbox can pair feedback with its exact input.
use alsa::poll::{self, Descriptors};
use alsa::seq::{EvNote, Event, EventType, PortCap, PortType, Seq};
use anyhow::{Context, Result};
use std::ffi::CString;
use std::time::{Duration, Instant};

struct VirtualPort {
    seq: Seq,
    port: i32,
}

impl VirtualPort {
    fn open(name: &str) -> Result<Self> {
        let seq = Seq::open(None, None, true).context("Opening the ALSA sequencer")?;
        seq.set_client_name(c"recordbox virtual controller")?;
        let caps = PortCap::READ | PortCap::SUBS_READ | PortCap::WRITE | PortCap::SUBS_WRITE;
        let port =
            seq.create_simple_port(&CString::new(name)?, caps, PortType::MIDI_GENERIC | PortType::APPLICATION)?;
        Ok(Self { seq, port })
    }

    fn send_note(&self, velocity: u8) -> Result<()> {
        let data = EvNote { channel: 0, note: 0x0b, velocity, ..Default::default() };
        let mut event = Event::new(EventType::Noteon, &data);
        event.set_source(self.port);
        event.set_subs();
        event.set_direct();
        self.seq.event_output_direct(&mut event)?;
        Ok(())
    }

    fn feedback_until(&self, until: Instant) -> Result<u64> {
        let mut input = self.seq.input();
        let mut fds = (&self.seq, Some(alsa::Direction::Capture)).get()?;
        let mut count = 0;
        while let Some(remaining) = until.checked_duration_since(Instant::now()) {
            match input.event_input() {
                Ok(event) => {
                    use EventType::*;
                    // ALSA also reports connections and disconnections on this port.
                    if matches!(
                        event.get_type(),
                        Noteon | Noteoff | Keypress | Controller | Control14 | Pgmchange | Chanpress | Pitchbend
                    ) {
                        count += 1;
                    }
                }
                Err(error) if error.errno() == libc::EINTR => continue,
                Err(error) if error.errno() == libc::EAGAIN => {
                    let timeout_ms = remaining.as_millis() as i32 + 1;
                    if let Err(error) = poll::poll(&mut fds, timeout_ms) {
                        if error.errno().abs() != libc::EINTR {
                            return Err(error.into());
                        }
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(count)
    }
}

pub(super) fn virtual_controller(name: &str, duration: Duration) -> Result<()> {
    let ctl = VirtualPort::open(name)?;
    eprintln!(
        "Virtual controller {name} at {}:{} is sending note 0B on channel 1 for {:.2} s",
        ctl.seq.client_id()?,
        ctl.port,
        duration.as_secs_f64()
    );
    let end = Instant::now() + duration;
    let mut received = 0;
    while Instant::now() < end {
        ctl.send_note(127)?;
        received += ctl.feedback_until(end.min(Instant::now() + Duration::from_millis(50)))?;
        ctl.send_note(0)?;
        received += ctl.feedback_until(end.min(Instant::now() + Duration::from_millis(950)))?;
    }
    println!("Received {received} channel feedback messages");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a Linux ALSA sequencer at /dev/snd/seq"]
    fn alsa_duplex_port_routes_both_directions_at_one_address() -> Result<()> {
        let name = "recordbox duplex test";
        let ctl = VirtualPort::open(name)?;
        let id = format!("{}:{}", ctl.seq.client_id()?, ctl.port);
        let input = midir::MidiInput::new("recordbox duplex test reader")?;
        let output = midir::MidiOutput::new("recordbox duplex test writer")?;
        let source = input.ports().into_iter().find(|p| p.id() == id).context("virtual input missing")?;
        let destinations = output.ports();
        let output_names: Vec<_> = destinations.iter().map(|p| output.port_name(p)).collect::<Result<_, _>>()?;
        let index = crate::midi_csv::paired_output(&input.port_name(&source)?, &output_names)
            .context("virtual feedback destination missing")?;
        assert_eq!(destinations[index].id(), id);
        let (tx, rx) = crossbeam_channel::bounded(2);
        let _reader = input
            .connect(
                &source,
                "test-notes",
                move |_, msg, _| {
                    let _ = tx.try_send(msg.to_vec());
                },
                (),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut writer = output.connect(&destinations[index], "test-led").map_err(|e| anyhow::anyhow!("{e}"))?;
        ctl.send_note(127)?;
        assert_eq!(rx.recv_timeout(Duration::from_secs(2))?, [0x90, 0x0b, 127]);
        writer.send(&[0x90, 0x0c, 127])?;
        assert_eq!(ctl.feedback_until(Instant::now() + Duration::from_millis(250))?, 1);
        Ok(())
    }
}
