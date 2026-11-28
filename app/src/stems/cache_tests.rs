use super::*;
use crate::disk_space::Space;
use std::cell::Cell;

const NAME: &str = "000000000000000000000001-htdemucs.stems";
const OTHER: &str = "000000000000000000000002-htdemucs_ft.stems";

fn parts(frames: usize, level: f32) -> Stems {
    Stems { len: frames, data: vec![level; frames * 8] }
}

#[test]
fn volume_reserve_counts_the_complete_replacement_and_never_evicts_on_failure() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let dest = dir.path().join(NAME);
    let old = dir.path().join(OTHER);
    let foreign = dir.path().join(".recordbox-stems-other.partial");
    std::fs::write(&foreign, b"someone else's incomplete work")?;
    let roomy = |_: &Path| Ok(Space { capacity: 1001, available: 1001 });
    for p in [&dest, &old] { save_checked(p, &parts(1, 0.25), || None, roomy)?; }
    let original = std::fs::read(&dest)?;

    // Ceil(1001 / 10) = 101, and the new two-frame cache needs 16 + 32 bytes.
    // Its predecessor is still on disk and cannot be credited as free space.
    for limit in [None, Some(0), Some(50_000_000_000)] {
        let error = save_checked(&dest, &parts(2, 0.5), || limit, |_| Ok(Space { capacity: 1001, available: 148 })).unwrap_err();
        assert!(error.to_string().contains("10%"));
        assert_eq!(std::fs::read(&dest)?, original);
        assert!(old.is_file());
    }
    let error = save_checked(&dest, &parts(2, 0.5), || Some(0),
        |_| Err(std::io::Error::other("cannot determine volume size"))).unwrap_err();
    assert!(format!("{error:#}").contains("cannot determine volume size"));
    assert_eq!(std::fs::read(&dest)?, original);
    assert!(old.is_file());

    // Exactly enough, with the headroom still present immediately before publish.
    let calls = Cell::new(0);
    save_checked(&dest, &parts(2, 0.5), || Some(0), |_| {
        let available = if calls.replace(calls.get() + 1) == 0 { 149 } else { 101 };
        Ok(Space { capacity: 1001, available })
    })?;
    assert_eq!(open_cache(&dest)?.1, 2);
    assert!(!old.exists());
    assert_eq!(std::fs::read(&foreign)?, b"someone else's incomplete work");
    Ok(())
}

#[test]
fn an_external_allocation_during_staging_or_fsync_leaves_no_partial_result() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let dest = dir.path().join(NAME);
    let unrelated_cache = dir.path().join(OTHER);
    let plenty = |_: &Path| Ok(Space { capacity: 1 << 30, available: 1 << 30 });
    for path in [&dest, &unrelated_cache] { save_checked(path, &parts(1, 0.25), || None, plenty)?; }
    let original = std::fs::read(&dest)?;
    for frames in [1, 600_000] {
        // The second probe is just before publication for the small file, after
        // roughly 8 MiB (with more PCM left to write) for the larger one.
        let calls = Cell::new(0);
        let res = save_checked(&dest, &parts(frames, -0.5), || Some(0), |_| {
            let available = if calls.replace(calls.get() + 1) == 0 { 1 << 30 } else { 1 << 20 };
            Ok(Space { capacity: 1 << 30, available })
        });
        assert!(res.unwrap_err().to_string().contains("10%"));
        assert_eq!(calls.get(), 2);
        assert_eq!(std::fs::read(&dest)?, original);
        assert!(unrelated_cache.is_file());
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 3, "two complete caches and the advisory lock");
    }
    Ok(())
}

#[test]
fn competing_writers_do_not_both_spend_the_same_disk_space() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let barrier = std::sync::Barrier::new(2);
    let volume = |folder: &Path| {
        let mut used = 0;
        for entry in std::fs::read_dir(folder)? { used += entry?.metadata()?.len(); }
        // Enough for one 32-byte save above the 100-byte reserve.
        Ok(Space { capacity: 1000, available: 150u64.saturating_sub(used) })
    };
    let write = |name| {
        barrier.wait();
        save_checked(&dir.path().join(name), &parts(1, 0.5), || None, volume)
    };
    let (a, b) = std::thread::scope(|s| {
        let first = s.spawn(|| write(NAME));
        let second = write(OTHER);
        (first.join().unwrap(), second)
    });
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(saved_entries(dir.path())?.len(), 1);
    Ok(())
}

#[test]
fn clearing_a_busy_cache_returns_immediately_and_failed_publication_discards_its_tempfile() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join(OTHER);
    let plenty = |_: &Path| Ok(Space { capacity: 1000, available: 1000 });
    save_checked(&old, &parts(1, 0.25), || None, plenty)?;
    let guard = cache_lock(dir.path())?;
    guard.lock()?;
    assert!(clear_unless_busy(dir.path()).unwrap_err().to_string().contains("busy"));
    assert!(old.is_file());
    drop(guard);

    let blocked = dir.path().join(NAME);
    std::fs::create_dir(&blocked)?;
    let error = save_checked(&blocked, &parts(1, 0.5), || Some(0), plenty).unwrap_err();
    assert!(error.to_string().contains("Publishing"));
    assert!(blocked.is_dir());
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 3);
    assert!(old.is_file());
    clear_unless_busy(dir.path())?;
    assert!(blocked.is_dir());
    assert!(!old.exists());
    Ok(())
}
