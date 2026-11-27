use super::*;
use std::{env, fs, process::Command};

const CHILD_HOME: &str = "RECORDBOX_TEST_CLEANUP_HOME";

fn scratch_parent() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        // CI can put TMPDIR under /var, which is intentionally an unclassified source path.
        // Its startup Data volume has an explicit system temp directory on supported macOS.
        let data_tmp = Path::new("/System/Volumes/Data/private/tmp");
        let checkout = env::current_dir().unwrap();
        return if checkout.starts_with("/Users") && env::var_os("GITHUB_ACTIONS").is_some() {
            checkout
        } else {
            data_tmp.into()
        };
    }
    #[cfg(windows)]
    {
        // Hosted runners can put their build checkout and TEMP on a separate drive.
        let temp = env::temp_dir();
        let windows = PathBuf::from(env::var_os("SystemRoot").unwrap());
        return if temp.components().next() == windows.components().next() {
            temp
        } else {
            windows.join("Temp")
        };
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    env::temp_dir()
}

#[test]
fn full_review_move_and_restore_in_isolated_profile() {
    let Some(home) = env::var_os(CHILD_HOME).map(PathBuf::from) else {
        let temp = tempfile::Builder::new()
            .prefix("recordbox-review-")
            .tempdir_in(scratch_parent())
            .unwrap();
        let home = temp.path();
        fs::write(home.join("fixture"), b"cleanup integration test").unwrap();
        // Isolate process-level directory settings from other parallel tests.
        let test = "collection_cleanup::tests::native_roundtrip::full_review_move_and_restore_in_isolated_profile";
        let child = Command::new(env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .current_dir(home)
            .env(CHILD_HOME, home)
            .env("HOME", home)
            .env("PREPDECK_DATA", home.join("private"))
            .env("APPDATA", home.join("roaming"))
            .env("LOCALAPPDATA", home.join("local"))
            .output()
            .unwrap();
        assert!(
            child.status.success() && home.join("completed").is_file(),
            "Isolated cleanup fixture failed\n{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    };
    assert_eq!(
        fs::canonicalize(&home).unwrap(),
        fs::canonicalize(".").unwrap()
    );
    assert_eq!(
        fs::read(home.join("fixture")).unwrap(),
        b"cleanup integration test"
    );
    assert_eq!(crate::db::data_dir_for_inspection(), home.join("private"));
    let music = home.join("Music");
    fs::create_dir(&music).unwrap();
    fs::create_dir(home.join("private")).unwrap();
    let library = home.join("private/library.db");
    let db = Db::open(&library).unwrap();
    let bytes = crate::player_ext::wav16_bytes(44_100, 2, &[0.25; 2048]);
    let import = |folder: &str, title: &str| {
        let parent = music.join(folder);
        fs::create_dir(&parent).unwrap();
        let path = parent.join(title);
        fs::write(&path, &bytes).unwrap();
        let mut track = crate::importer::read_track(&path);
        assert_eq!(track.sample_rate, 44_100);
        // A run spanning midnight should still start with identical catalog dates.
        track.date_added = "2026-01-01".into();
        track.date_created.clone_from(&track.date_added);
        (db.insert_local_file(&track).unwrap(), path)
    };
    let (keeper, original) = import("first", "Mix.wav");
    let (duplicate, copy) = import("second", "Mix.wav");
    let (missing, absent) = import("third", "Absent.wav");
    let (protected, sentinel) = import("Dropbox", "Protected.wav");
    let moved = home.join("moved-outside-library.wav");
    fs::rename(&absent, &moved).unwrap();
    let report = review_path_cancellable(&library, true, &false.into()).unwrap();
    let selected = BTreeSet::from([missing, duplicate]);
    if cfg!(any(windows, target_os = "macos")) {
        let row = |id| report.rows.iter().find(|row| row.id == id).unwrap();
        assert_eq!(
            row(missing).candidate,
            Some(Candidate::Missing),
            "{:?}",
            row(missing)
        );
        assert_eq!(
            row(duplicate).candidate,
            Some(Candidate::Duplicate { keeper }),
            "{:?}",
            row(duplicate)
        );
        assert_eq!(row(protected).status, PathStatus::Cloud);
        assert_eq!(report.hashed_files, 2);
        assert!(store::apply(
            &library,
            &report,
            &BTreeSet::from([protected]),
            true,
            "MOVE 1"
        )
        .is_err());
        // A file reappearing after the review must stop the entire operation before backup.
        fs::rename(&moved, &absent).unwrap();
        assert!(store::apply(&library, &report, &selected, true, "MOVE 2").is_err());
        assert!(!home.join("private/cleanup-backups").exists());
        fs::rename(&absent, &moved).unwrap();

        // A newly linked stem source must also invalidate the earlier duplicate review.
        let link = crate::stem_export::link_key(protected);
        db.set_setting_checked(&link, &format!("{duplicate}:vocals")).unwrap();
        assert!(store::apply(&library, &report, &selected, true, "MOVE 2").is_err());
        assert!(db.trash_list().unwrap().is_empty());
        assert!(!home.join("private/cleanup-backups").exists());
        db.conn.execute("DELETE FROM settings WHERE key=?1", [link]).unwrap();

        let archive = store::apply(&library, &report, &selected, true, "MOVE 2").unwrap();
        assert_eq!(crate::backup::read_manifest(&archive).unwrap().tracks, 4);
        assert!(db.track(keeper).is_some() && db.track(protected).is_some());
        assert!(db.track(missing).is_none() && db.track(duplicate).is_none());
        let mut restored = BTreeSet::new();
        for (trash, ..) in db.trash_list().unwrap() {
            restored.insert(db.restore_trash(trash).unwrap().unwrap());
        }
        assert_eq!(restored, selected);
        assert_eq!(db.track(missing).unwrap().path, absent.to_str().unwrap());
        assert_eq!(db.track(duplicate).unwrap().path, copy.to_str().unwrap());
    } else {
        assert!(!report.can_apply && report.hashed_files == 0);
        assert!(store::apply(&library, &report, &selected, true, "MOVE 2").is_err());
        assert!(!home.join("private/cleanup-backups").exists());
    }
    assert!(!absent.exists());
    for path in [original, copy, moved, sentinel] {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(db.trash_list().unwrap().is_empty());
    assert_eq!(db.all_tracks().unwrap().len(), 4);
    fs::write(home.join("completed"), []).unwrap();
}
