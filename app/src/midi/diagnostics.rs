//! Bounded command-line checks for controller input and virtual feedback.
use anyhow::{anyhow, ensure, Result};
use std::time::Duration;

fn arguments(args: &[String]) -> Result<(&str, Duration)> {
    ensure!((1..=2).contains(&args.len()), "Supply a port name and optional duration in seconds");
    let name = args[0].as_str();
    ensure!(!name.trim().is_empty(), "Supply a nonempty port name");
    let secs = args.get(1).map(|s| s.parse::<f64>()).transpose()?.unwrap_or(10.0);
    Ok((name, duration(secs)?))
}

pub(super) fn duration(secs: f64) -> Result<Duration> {
    ensure!((0.05..=120.0).contains(&secs), "Duration must be between 0.05 and 120 seconds");
    Ok(Duration::from_secs_f64(secs))
}

pub(crate) fn cli(args: &[String]) -> Option<Result<()>> {
    let mode = args.first()?.as_str();
    match mode {
        "--midi-ports" => Some(if args.len() == 1 { ports() } else { Err(anyhow!("--midi-ports takes no arguments")) }),
        "--midi-probe" | "--midi-virtual" => Some(arguments(&args[1..]).and_then(|(port, time)| {
            if mode == "--midi-probe" { super::read_input(port, time) } else { virtual_controller(port, time) }
        })),
        _ => None,
    }
}

fn ports() -> Result<()> {
    let input = midir::MidiInput::new("recordbox list")?;
    let output = midir::MidiOutput::new("recordbox list")?;
    println!("Inputs:");
    for port in input.ports() { println!("  {}", input.port_name(&port)?); }
    println!("Outputs:");
    for port in output.ports() { println!("  {}", output.port_name(&port)?); }
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::virtual_controller;

#[cfg(target_os = "macos")]
fn virtual_controller(name: &str, duration: Duration) -> Result<()> {
    use midir::os::unix::{VirtualInput, VirtualOutput};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::sync::Arc;
    use std::time::Instant;

    let received = Arc::new(AtomicU64::new(0));
    let counter = received.clone();
    let _feedback = midir::MidiInput::new("recordbox virtual feedback")?
        .create_virtual(name, move |_, _, _| { counter.fetch_add(1, Relaxed); }, ())
        .map_err(|e| anyhow!("Could not create virtual feedback port: {e}"))?;
    let mut controller = midir::MidiOutput::new("recordbox virtual controller")?
        .create_virtual(name).map_err(|e| anyhow!("Could not create virtual input source: {e}"))?;
    eprintln!("Virtual controller {name} is sending note 0B on channel 1 for {:.2} s", duration.as_secs_f64());
    let end = Instant::now() + duration;
    while Instant::now() < end {
        controller.send(&[0x90, 0x0b, 0x7f])?;
        std::thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
        controller.send(&[0x90, 0x0b, 0])?;
        std::thread::sleep(Duration::from_millis(950).min(end.saturating_duration_since(Instant::now())));
    }
    println!("Received {} feedback messages", received.load(Relaxed));
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn virtual_controller(_name: &str, _duration: Duration) -> Result<()> {
    anyhow::bail!("Virtual MIDI port creation is available on Linux and macOS. Use --midi-ports or --midi-probe on this platform.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_commands_validate_their_arguments_before_opening_any_port() {
        for s in ["NaN", "inf", "-1", "0", "0.0001", "121", "hours"] {
            let args = ["--midi-probe".into(), "controller".into(), s.into()];
            assert!(cli(&args).unwrap().is_err(), "{s}");
        }
        assert!(cli(&["--midi-ports".into(), "extra".into()]).unwrap().is_err());
        assert!(cli(&["--midi-virtual".into()]).unwrap().is_err());
        assert!(cli(&["--help".into()]).is_none());
        assert_eq!(arguments(&["controller".into()]).unwrap().1, Duration::from_secs(10));
    }
}
