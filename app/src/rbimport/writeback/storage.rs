//! The live write boundary. SQLite owns the database transaction. Separate ANLZ files
//! require a durable before/after journal, even when a batch changes only one grid.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{
    backup::{Backup, StepResult},
    types::ValueRef,
    Connection, OpenFlags, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};
use tempfile::NamedTempFile;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem as winfs;

use crate::export::usb::sha256_file;
use crate::rbimport::{check_schema, MASTER_DB_KEY};
use super::RecoveryFiles;

const MARKER: &str = ".recordbox-writeback-pending.json";
const MANIFEST: &str = "grid-recovery.json";
const STAGE_PREFIX: &str = "recordbox-transaction-";
const LOCK: &str = ".recordbox-writeback.lock";
const MAX_MARKER: u64 = 256 * 1024;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct FileChange {
    /// Relative to the native library, including the leading share directory.
    pub relative_path: PathBuf,
    pub expected_sha256: [u8; 32],
    pub contents: Vec<u8>,
}

#[derive(Deserialize, Serialize)]
struct GridFile {
    path: PathBuf,
    before: [u8; 32],
    after: [u8; 32],
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
enum CatalogProof {
    #[default]
    EncryptedBackup,
    LockedTransactionV1,
}

#[derive(Deserialize, Serialize)]
struct GridJournal {
    // Existing manifests always used a full encrypted copy and keep that verification path.
    #[serde(default)]
    catalog_proof: CatalogProof,
    source: PathBuf,
    source_file: NativeFileId,
    before_db: [u8; 32],
    after_db: [u8; 32],
    files: Vec<GridFile>,
}

#[derive(Deserialize, Serialize)]
struct Pending {
    directory: PathBuf,
    manifest_sha256: [u8; 32],
}

pub fn pending(source_dir: &Path) -> bool {
    match fs::symlink_metadata(source_dir.join(MARKER)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        _ => true,
    }
}

fn app_process(command: &str) -> bool {
    let name = command
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = name.trim_end_matches(".exe");
    name == "rekordbox"
        || name.starts_with("rekordboxagent")
        || name.starts_with("rekordbox agent")
        || name.starts_with("rekordbox helper")
        || name.starts_with("rekordbox (")
        || name
            .strip_prefix("rekordbox ")
            .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

fn ensure_apps_closed() -> Result<()> {
    check_process_list(app_process)
}

fn check_process_list(blocked: impl Fn(&str) -> bool) -> Result<()> {
    #[cfg(windows)]
    let output = {
        use std::os::windows::process::CommandExt;
        let system =
            std::env::var_os("SystemRoot").context("cannot locate the Windows process list")?;
        Command::new(PathBuf::from(system).join("System32/tasklist.exe"))
            .args(["/FO", "CSV", "/NH"])
            .creation_flags(0x08000000)
            .output()
    };
    #[cfg(unix)]
    let output = Command::new("/bin/ps").args(["-A", "-o", "comm="]).output();
    let output = output.context("cannot check whether rekordbox or rekordboxAgent is running")?;
    ensure!(
        output.status.success(),
        "could not inspect running applications"
    );
    let lines = String::from_utf8_lossy(&output.stdout);
    #[cfg(windows)]
    let names = lines
        .lines()
        .filter_map(|s| s.strip_prefix('"')?.split('"').next());
    #[cfg(unix)]
    let names = lines.lines().filter(|s| !s.trim().is_empty());
    let mut listed = false;
    for name in names {
        listed = true;
        ensure!(
            !blocked(name),
            "close rekordbox and rekordboxAgent before writing or recovering this library"
        );
    }
    ensure!(
        listed,
        "could not identify any applications in the system process list"
    );
    Ok(())
}

// Network filesystems and cloud-backed library folders can admit writers outside
// the local process list. Restrict the pilot to known local filesystems.
fn local_volume(path: &Path) -> Result<()> {
    for part in path.components() {
        let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
        ensure!(
            name != "dropbox"
                && !name.ends_with(" dropbox")
                && !name.starts_with("dropbox (")
                && name != "onedrive"
                && !name.starts_with("onedrive - ")
                && name != "mobile documents"
                && name != "cloudstorage"
                && name != "google drive"
                && name != "icloud drive",
            "choose a local library and recovery folder outside cloud-synchronized directories"
        );
    }
    #[cfg(unix)]
    let local = {
        use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
        let path = CString::new(path.as_os_str().as_bytes())?;
        let mut status = MaybeUninit::<libc::statfs>::uninit();
        // statfs initializes the whole output structure on success.
        if unsafe { libc::statfs(path.as_ptr(), status.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error()).context("checking the library filesystem");
        }
        let status = unsafe { status.assume_init() };
        #[cfg(target_os = "macos")]
        {
            status.f_flags & libc::MNT_LOCAL as u32 != 0
        }
        #[cfg(target_os = "linux")]
        {
            // ext, btrfs, XFS, tmpfs, overlayfs, F2FS, ZFS and native NTFS.
            matches!(
                status.f_type as u32,
                0xef53
                    | 0x9123683e
                    | 0x58465342
                    | 0x01021994
                    | 0x794c7630
                    | 0xf2f52010
                    | 0x2fc12fc1
                    | 0x5346544e
            )
        }
    };
    #[cfg(windows)]
    let local = {
        use std::path::Prefix;
        let drive = match path.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(d),
                _ => None,
            },
            _ => None,
        };
        drive.is_some_and(|d| {
            let root = [d as u16, b':' as u16, b'\\' as u16, 0];
            unsafe { winfs::GetDriveTypeW(root.as_ptr()) == 3 } // DRIVE_FIXED
        })
    };
    ensure!(
        local,
        "write-back requires a supported local filesystem for {}",
        path.display()
    );
    Ok(())
}

fn regular(path: &Path) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path).with_context(|| format!("inspecting {}", path.display()))?;
    ensure!(
        m.is_file() && !linked(&m),
        "expected a regular unlinked file at {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            m.nlink() == 1,
            "hard-linked file is outside the write-back pilot: {}",
            path.display()
        );
    }
    Ok(m)
}

fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & winfs::FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        } // junction or other reparse point
    }
    m.file_type().is_symlink()
}

fn grid_target(source: &Path, relative: &Path) -> Result<PathBuf> {
    let mut parts = relative.components();
    ensure!(
        parts.next() == Some(Component::Normal("share".as_ref())),
        "analysis path must be inside the native share directory"
    );
    let mut target = source.join("share");
    for part in parts {
        let Component::Normal(name) = part else {
            bail!("invalid relative analysis path");
        };
        let parent = fs::symlink_metadata(&target)?;
        ensure!(
            parent.is_dir() && !linked(&parent),
            "linked or invalid analysis directory"
        );
        target.push(name);
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    ensure!(
        name.starts_with("ANLZ")
            && [".DAT", ".EXT", ".2EX"]
                .iter()
                .any(|ext| name.ends_with(ext)),
        "unexpected native analysis filename"
    );
    regular(&target)?;
    local_volume(&target)?;
    Ok(target)
}

// Also bind the operation to this particular master.db. A backup restored at
// the same path or an externally replaced library needs a fresh review.
#[derive(Deserialize, Serialize, PartialEq, Eq, Hash)]
struct NativeFileId {
    volume: u64,
    file: [u8; 16],
}

fn native_id(path: &Path) -> Result<NativeFileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Avoid opening a separate fd to the live DB: closing it can drop
        // process-owned POSIX locks that SQLite has on that inode.
        let m = regular(path)?;
        let mut file = [0u8; 16];
        file[..8].copy_from_slice(&m.ino().to_le_bytes());
        Ok(NativeFileId {
            volume: m.dev() as u64,
            file,
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        fn query<T: Default>(f: &File, class: winfs::FILE_INFO_BY_HANDLE_CLASS) -> Result<T> {
            let mut value = T::default();
            let ok = unsafe {
                winfs::GetFileInformationByHandleEx(
                    f.as_raw_handle(),
                    class,
                    (&mut value as *mut T).cast(),
                    std::mem::size_of::<T>() as u32,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error()).context("identifying native file");
            }
            Ok(value)
        }
        regular(path)?;
        let f = File::open(path)?;
        let id: winfs::FILE_ID_INFO = query(&f, winfs::FileIdInfo)?;
        let info: winfs::FILE_STANDARD_INFO = query(&f, winfs::FileStandardInfo)?;
        ensure!(
            info.NumberOfLinks == 1 && !info.DeletePending,
            "native file is linked or pending deletion"
        );
        Ok(NativeFileId {
            volume: id.VolumeSerialNumber,
            file: id.FileId.Identifier,
        })
    }
}

fn check_native_files(source: &Path, transaction_open: bool) -> Result<()> {
    regular(&source.join("master.db"))?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let path = source.join(format!("master.db{suffix}"));
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            result => {
                result?;
            }
        }
        let m = regular(&path)?;
        ensure!(
            transaction_open || suffix != "-journal" || m.len() == 0,
            "active rollback journal; close rekordbox cleanly and try again"
        );
    }
    Ok(())
}

fn native(source: &Path, mode: OpenFlags) -> Result<Connection> {
    let conn = Connection::open_with_flags(source, mode | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
    conn.busy_timeout(Duration::ZERO)?;
    conn.pragma_update(None, "key", MASTER_DB_KEY)?;
    conn.pragma_update(None, "trusted_schema", false)?;
    Ok(conn)
}

fn integrity(db: &Connection) -> Result<()> {
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        integrity == "ok",
        "library failed SQLite integrity validation: {integrity}"
    );
    let mut cipher = db.prepare("PRAGMA cipher_integrity_check")?;
    ensure!(
        cipher.query([])?.next()?.is_none(),
        "library failed SQLCipher page authentication"
    );
    check_schema(db)
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    // Windows publication below uses MOVEFILE_WRITE_THROUGH.
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

fn publish(temp: NamedTempFile, target: &Path, replace: bool) -> Result<()> {
    temp.as_file().sync_all()?;
    #[cfg(unix)]
    if replace {
        temp.persist(target)?;
    } else {
        temp.persist_noclobber(target)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // All parents are canonical absolute paths (including verbatim/long paths).
        let old = temp
            .path()
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        let new = target
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>();
        // NamedTempFile uses FILE_ATTRIBUTE_TEMPORARY on Windows. Clear just
        // that hint and flush before making a persistent marker or sidecar.
        let attributes = unsafe { winfs::GetFileAttributesW(old.as_ptr()) };
        if attributes == winfs::INVALID_FILE_ATTRIBUTES {
            return Err(std::io::Error::last_os_error()).context("checking staged file attributes");
        }
        if attributes & winfs::FILE_ATTRIBUTE_TEMPORARY != 0 {
            let persistent = attributes & !winfs::FILE_ATTRIBUTE_TEMPORARY;
            let persistent = if persistent == 0 {
                winfs::FILE_ATTRIBUTE_NORMAL
            } else {
                persistent
            };
            if unsafe { winfs::SetFileAttributesW(old.as_ptr(), persistent) } == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("making staged file persistent");
            }
            temp.as_file().sync_all()?;
        }
        let flag = if replace {
            winfs::MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
        if unsafe {
            winfs::MoveFileExW(
                old.as_ptr(),
                new.as_ptr(),
                winfs::MOVEFILE_WRITE_THROUGH | flag,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error()).context("publishing write-back file");
        }
    }
    sync_directory(
        target
            .parent()
            .context("missing parent for write-back file")?,
    )
}

fn save_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = NamedTempFile::new_in(path.parent().context("missing recovery directory")?)?;
    temp.write_all(bytes)?;
    publish(temp, path, false)
}

fn backup_database(source: &Path, directory: &Path) -> Result<()> {
    let reader = native(&source.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let path = directory.join("master.db");
    {
        let mut copy = native(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        {
            let backup = Backup::new(&reader, &mut copy)?;
            loop {
                match backup.step(256)? {
                    StepResult::Done => break,
                    StepResult::More => continue,
                    _ => bail!("the database became busy while creating the recovery copy"),
                }
            }
        }
        let mode: String = copy.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        ensure!(
            mode.eq_ignore_ascii_case("delete"),
            "cannot consolidate recovery database"
        );
    }
    fs::OpenOptions::new().write(true).open(&path)?.sync_all()?;
    integrity(&native(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?)?;
    sync_directory(directory)
}

// A grid batch spans two different storage systems. Hash all persisted rows,
// schema and metadata of the private backup and the staged SQL transaction.
// During recovery even an unrelated subsequent database edit must stop replay.
fn logical_digest(db: &Connection) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    fn feed(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn rows(db: &Connection, sql: &str, hash: &mut Sha256) -> Result<()> {
        let mut query = db.prepare(sql)?;
        let cols = query.column_count();
        hash.update((cols as u64).to_le_bytes());
        let mut result = query.query([])?;
        while let Some(row) = result.next()? {
            hash.update([1]);
            for i in 0..cols {
                match row.get_ref(i)? {
                    ValueRef::Null => hash.update([0]),
                    ValueRef::Integer(v) => {
                        hash.update([1]);
                        hash.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        hash.update([2]);
                        hash.update(v.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(v) => {
                        hash.update([3]);
                        feed(hash, v);
                    }
                    ValueRef::Blob(v) => {
                        hash.update([4]);
                        feed(hash, v);
                    }
                }
            }
        }
        hash.update([0]);
        Ok(())
    }
    rows(db, "PRAGMA main.user_version", &mut hash)?;
    rows(db, "PRAGMA main.application_id", &mut hash)?;
    rows(
        db,
        "SELECT type,name,tbl_name,sql FROM main.sqlite_schema ORDER BY type,name",
        &mut hash,
    )?;
    let mut stmt = db.prepare("SELECT name,type,wr FROM pragma_table_list WHERE schema='main' AND name <> 'sqlite_schema' ORDER BY name")?;
    let tables = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })?;
    for table in tables {
        let (name, kind, without_rowid) = table?;
        ensure!(
            kind != "virtual" && kind != "shadow",
            "virtual tables are outside the grid recovery pilot"
        );
        if kind != "table" {
            continue;
        }
        feed(&mut hash, name.as_bytes());
        // A table scan yields rowid order or primary-key order for WITHOUT ROWID.
        let quoted = name.replace('"', "\"\"");
        let columns = if without_rowid {
            "*"
        } else {
            let shadowed: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_xinfo(?1) WHERE lower(name)='_rowid_')",
                [&name],
                |r| r.get(0),
            )?;
            ensure!(
                !shadowed,
                "cannot uniquely identify the rows of {name} for grid recovery"
            );
            "_rowid_,*"
        };
        rows(
            db,
            &format!("SELECT {columns} FROM main.\"{quoted}\" NOT INDEXED"),
            &mut hash,
        )?;
    }
    Ok(hash.finalize().into())
}

fn backup_parent(source: &Path, root: &Path) -> Result<PathBuf> {
    let requested = std::path::absolute(root)?;
    let existing = requested
        .ancestors()
        .find(|p| p.exists())
        .context("missing recovery parent")?
        .canonicalize()?;
    ensure!(
        !existing.starts_with(source),
        "choose a recovery folder outside the native library"
    );
    local_volume(&existing)?;
    fs::create_dir_all(&requested)?;
    let root = requested.canonicalize()?;
    ensure!(
        !root.starts_with(source),
        "recovery folder overlaps the native library"
    );
    local_volume(&root)?;
    Ok(root)
}

fn stage_grids(
    source: &Path,
    directory: &Path,
    db: &Connection,
    changes: Vec<FileChange>,
    locked_before: Option<[u8; 32]>,
) -> Result<GridJournal> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for (i, change) in changes.into_iter().enumerate() {
        let path = grid_target(source, &change.relative_path)?;
        ensure!(
            seen.insert(native_id(&path)?),
            "duplicate native analysis file in selection"
        );
        let before = sha256_file(&path)?;
        ensure!(
            before == change.expected_sha256,
            "native grid changed since review: {}",
            path.display()
        );
        let after: [u8; 32] = Sha256::digest(&change.contents).into();
        ensure!(before != after, "redundant analysis-file replacement");
        let copy = directory.join(format!("{i}.original"));
        {
            let mut temp = NamedTempFile::new_in(directory)?;
            std::io::copy(&mut File::open(&path)?, temp.as_file_mut())?;
            publish(temp, &copy, false)?;
        }
        ensure!(
            sha256_file(&copy)? == before,
            "analysis file changed while backing it up"
        );
        let edited = directory.join(format!("{i}.reviewed"));
        save_new(&edited, &change.contents)?;
        ensure!(
            sha256_file(&edited)? == after,
            "reviewed grid copy failed verification"
        );
        files.push(GridFile {
            path: change.relative_path,
            before,
            after,
        });
    }
    let (catalog_proof, before_db) = match locked_before {
        Some(before) => (CatalogProof::LockedTransactionV1, before),
        None => (CatalogProof::EncryptedBackup, logical_digest(&native(
            &directory.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?)?),
    };
    let after_db = logical_digest(db)?;
    ensure!(
        before_db != after_db,
        "grid update requires a corresponding native database revision"
    );
    Ok(GridJournal {
        catalog_proof,
        source: source.to_owned(),
        source_file: native_id(&source.join("master.db"))?,
        before_db,
        after_db,
        files,
    })
}

fn copy_grids(journal: &GridJournal, directory: &Path, forward: bool) -> Result<()> {
    // Preflight the complete set, including recovery contents, before the first rename.
    let mut staged = Vec::new();
    for (i, entry) in journal.files.iter().enumerate() {
        let (old, want, side) = if forward {
            (entry.before, entry.after, "reviewed")
        } else {
            (entry.after, entry.before, "original")
        };
        let target = grid_target(&journal.source, &entry.path)?;
        let current = sha256_file(&target)?;
        ensure!(
            current == old || current == want,
            "analysis file has unrecognized edits: {}",
            target.display()
        );
        if current == want {
            continue;
        }
        let saved = directory.join(format!("{i}.{side}"));
        regular(&saved)?;
        ensure!(
            sha256_file(&saved)? == want,
            "recovery grid checksum mismatch"
        );
        ensure!(
            !regular(&target)?.permissions().readonly(),
            "native analysis file is read-only: {}",
            target.display()
        );
        staged.push((saved, entry, old, want));
    }
    // Only one replacement file descriptor and one extra same-volume file are
    // needed at once, including on systems with a small open-file limit.
    for (saved, entry, old, want) in staged {
        let target = grid_target(&journal.source, &entry.path)?;
        let mut temp = NamedTempFile::new_in(target.parent().context("missing analysis folder")?)?;
        std::io::copy(&mut File::open(saved)?, temp.as_file_mut())?;
        ensure!(
            sha256_file(temp.path())? == want,
            "staged grid checksum mismatch"
        );
        fs::set_permissions(temp.path(), regular(&target)?.permissions())?;
        ensure!(
            sha256_file(&target)? == old,
            "native analysis changed during publication"
        );
        publish(temp, &target, true)?;
        ensure!(
            sha256_file(&target)? == want,
            "published analysis failed verification"
        );
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = regular(path)?;
    ensure!(
        metadata.len() <= limit,
        "oversized recovery metadata at {}",
        path.display()
    );
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "oversized recovery metadata");
    Ok(bytes)
}

// All Recordbox profiles using this native library share the same advisory lock,
// including the interval between SQLite commit and clearing a pending file batch.
// Leave the lock inode in place even after unlocking it.
fn operation_lock(source: &Path, recovering: bool) -> Result<File> {
    ensure_apps_closed()?;
    local_volume(source)?;
    check_native_files(source, recovering)?;
    let path = source.join(LOCK);
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(winfs::FILE_SHARE_READ | winfs::FILE_SHARE_WRITE)
            .custom_flags(winfs::FILE_FLAG_OPEN_REPARSE_POINT); // share read/write, open the reparse point itself.
    }
    let lock = options
        .open(&path)
        .context("opening the Recordbox write-back lock")?;
    let current = regular(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = lock.metadata()?;
        ensure!(
            current.ino() == opened.ino() && current.dev() == opened.dev(),
            "write-back lock file changed while opening it"
        );
    }
    #[cfg(windows)]
    let _ = current;
    lock.try_lock()
        .context("another Recordbox write-back or recovery operation is using this library")?;
    Ok(lock)
}

fn writer(source: &Path) -> Result<Connection> {
    let db = native(&source.join("master.db"), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let journal: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    ensure!(
        matches!(journal.as_str(), "wal" | "delete" | "truncate" | "persist"),
        "native library requires a recoverable SQLite journal mode"
    );
    db.pragma_update(None, "synchronous", "FULL")?;
    db.pragma_update(None, "foreign_keys", true)?;
    Ok(db)
}

/// The callback re-reads previewed values on this connection. All its writes
/// participate in this transaction and must not commit or open another writer.
#[cfg(test)]
fn apply<T>(
    source_dir: &Path,
    backup_root: &Path,
    under_lock: impl FnOnce(&Connection) -> Result<(T, Vec<FileChange>)>,
) -> Result<(T, PathBuf)> {
    let (result, directory) = apply_with_policy(source_dir, backup_root, RecoveryCopy::Keep, under_lock)?;
    Ok((result, directory.backup.context("missing requested recovery backup")?))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryCopy {
    Keep,
    TemporaryForGrids,
    DatabaseTransaction,
}

pub(super) fn apply_with_policy<T>(
    source_dir: &Path,
    backup_root: &Path,
    policy: RecoveryCopy,
    under_lock: impl FnOnce(&Connection) -> Result<(T, Vec<FileChange>)>,
) -> Result<(T, RecoveryFiles)> {
    let source = source_dir.canonicalize()?;
    let _guard = operation_lock(&source, false)?;
    let identity = native_id(&source.join("master.db"))?;
    let mut db = writer(&source)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context(
            "cannot acquire the exclusive SQLite writer slot; close all rekordbox library writers",
        )?;
    ensure!(
        !pending(&source),
        "previous write-back needs recovery; use Recover interrupted write-back first"
    );
    integrity(&tx)?;
    // A trigger can propagate a single reviewed field edit to unrelated rows.
    // Native triggers need schema-specific validation before this writer can use them.
    let mut triggers = tx.prepare("SELECT name,tbl_name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name) IN ('djmdcontent','djmdcue','contentcue','contentfile','agentregistry') ORDER BY name LIMIT 1")?;
    let trigger = triggers.query_row([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
    match trigger {
        Ok((name, table)) => {
            bail!("unverified database trigger {name} on {table}; no write-back was applied")
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => (),
        Err(error) => return Err(error.into()),
    }
    drop(triggers);
    let verify_before_commit = || -> Result<()> {
        check_native_files(&source, true)?;
        ensure!(native_id(&source.join("master.db"))? == identity, "native library file changed during review");
        ensure_apps_closed()
    };
    if policy == RecoveryCopy::DatabaseTransaction {
        let (result, grids) = under_lock(&tx)?;
        ensure!(grids.is_empty(), "a native analysis-file edit requires a recovery journal");
        verify_before_commit()?;
        tx.commit().context("committing the native database transaction")?;
        return Ok((result, RecoveryFiles::default()));
    }

    let root = backup_parent(&source, backup_root)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(if policy == RecoveryCopy::Keep { "rekordbox-" } else { STAGE_PREFIX });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let backup = builder.tempdir_in(&root)?;
    let locked_before = if policy == RecoveryCopy::Keep {
        backup_database(&source, backup.path())?;
        ensure!(
            native_id(&source.join("master.db"))? == identity,
            "native library file changed during backup"
        );
        let receipt = serde_json::json!({
            "format": "recordbox-native-recovery-v1",
            "captured_unix_seconds": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
            "source": source.join("master.db"),
            "source_file_identity": &identity,
            "recovery_database": "master.db",
            "sha256": sha256_file(&backup.path().join("master.db"))?.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "includes_committed_wal": true,
        });
        save_new(
            &backup.path().join("backup.json"),
            &serde_json::to_vec_pretty(&receipt)?,
        )?;
        None
    } else {
        // BEGIN IMMEDIATE reserves the writer slot. This digest includes committed WAL
        // frames and precedes every reviewed SQL edit. SQLite owns database rollback.
        Some(logical_digest(&tx)?)
    };
    let (result, grids) = under_lock(&tx)?;
    verify_before_commit()?;
    // Reopening a backup after a successful database-only operation always
    // yields its committed, pre-edit state, including the original WAL records.
    if grids.is_empty() {
        if policy == RecoveryCopy::TemporaryForGrids {
            tx.commit().context("committing the native database transaction")?;
            return Ok((result, RecoveryFiles::default()));
        }
        let directory = backup.keep();
        sync_directory(&root)?;
        tx.commit().with_context(|| {
            format!(
                "database commit failed; verified recovery copy: {}",
                directory.display()
            )
        })?;
        return Ok((result, RecoveryFiles { backup: Some(directory), remaining_staging: None }));
    }

    let journal = stage_grids(&source, backup.path(), &tx, grids, locked_before)?;
    let manifest = serde_json::to_vec_pretty(&journal)?;
    ensure!(
        manifest.len() as u64 <= MAX_MANIFEST,
        "selection exceeds the recoverable grid batch size"
    );
    save_new(&backup.path().join(MANIFEST), &manifest)?;
    sync_directory(&root)?;
    let intent = Pending {
        directory: backup.path().to_owned(),
        manifest_sha256: Sha256::digest(&manifest).into(),
    };
    // Once the marker might be visible, even an fsync error must preserve its recovery set.
    let directory = backup.keep();
    save_new(&source.join(MARKER), &serde_json::to_vec(&intent)?)?;
    let commit = || -> Result<()> {
        ensure_apps_closed()?;
        ensure!(
            native_id(&source.join("master.db"))? == identity,
            "native library file changed before grid publication"
        );
        copy_grids(&journal, &directory, true)?;
        ensure_apps_closed()?;
        ensure!(
            native_id(&source.join("master.db"))? == identity,
            "native library file changed before commit"
        );
        tx.commit()?;
        fs::remove_file(source.join(MARKER))?;
        sync_directory(&source)
    };
    commit().with_context(|| {
        format!(
            "Recover interrupted write-back before continuing. Recovery files: {}",
            directory.display()
        )
    })?;
    Ok((result, finish_recovery_directory(directory, &journal)))
}

/// A successful transaction may retire only its own recognized staging files. Never
/// recursively delete a directory recovered from a marker. Keep and report the path
/// if another file appeared or the OS could not remove a staged file.
fn finish_recovery_directory(directory: PathBuf, journal: &GridJournal) -> RecoveryFiles {
    if journal.catalog_proof == CatalogProof::EncryptedBackup {
        return RecoveryFiles { backup: Some(directory), remaining_staging: None };
    }
    let retire = || -> Result<()> {
        let is_stage = directory.file_name().and_then(|s| s.to_str()).is_some_and(|name| name.starts_with(STAGE_PREFIX));
        ensure!(is_stage, "unrecognized recovery directory");
        let mut owned = HashSet::from([PathBuf::from(MANIFEST)]);
        for i in 0..journal.files.len() {
            owned.insert(format!("{i}.original").into());
            owned.insert(format!("{i}.reviewed").into());
        }
        let actual = fs::read_dir(&directory)?.map(|entry| entry.map(|e| PathBuf::from(e.file_name())))
            .collect::<std::io::Result<HashSet<_>>>()?;
        ensure!(actual == owned, "unrecognized recovery files");
        for name in &owned { regular(&directory.join(name))?; }
        for name in owned { fs::remove_file(directory.join(name))?; }
        fs::remove_dir(&directory)?;
        Ok(())
    };
    RecoveryFiles { backup: None, remaining_staging: retire().err().map(|_| directory) }
}

/// Complete an interrupted grid publication according to the database state
/// that actually committed. Never replace a file outside either recorded state.
pub fn recover(source_dir: &Path, backup_root: &Path) -> Result<RecoveryFiles> {
    let source = source_dir.canonicalize()?;
    let _guard = operation_lock(&source, true)?;
    let marker = source.join(MARKER);
    let pending: Pending = serde_json::from_slice(&read_bounded(&marker, MAX_MARKER)?)?;
    let directory = pending.directory.canonicalize()?;
    let root = backup_root.canonicalize()?;
    ensure!(
        directory.parent() == Some(root.as_path()),
        "expected recovery folder under {}; recorded {}",
        root.display(),
        directory.display()
    );
    local_volume(&directory)?;
    let manifest = read_bounded(&directory.join(MANIFEST), MAX_MANIFEST)?;
    let digest: [u8; 32] = Sha256::digest(&manifest).into();
    ensure!(
        digest == pending.manifest_sha256,
        "recovery manifest checksum mismatch"
    );
    let journal: GridJournal = serde_json::from_slice(&manifest)?;
    ensure!(
        journal.source == source,
        "recovery belongs to a different library"
    );
    ensure!(
        native_id(&source.join("master.db"))? == journal.source_file,
        "the native library file was replaced; inspect the recovery files before proceeding"
    );
    ensure!(journal.before_db != journal.after_db && !journal.files.is_empty(), "invalid recovery boundaries");
    match journal.catalog_proof {
        CatalogProof::EncryptedBackup => {
            regular(&directory.join("master.db"))?;
            let pre = native(&directory.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            integrity(&pre)?;
            ensure!(logical_digest(&pre)? == journal.before_db, "recovery database has changed");
        }
        CatalogProof::LockedTransactionV1 => {
            ensure!(directory.file_name().and_then(|s| s.to_str()).is_some_and(|n| n.starts_with(STAGE_PREFIX)),
                "unexpected location for a temporary analysis journal");
        }
    }
    // Only a verified, pending operation may ask SQLite to resolve a hot
    // rollback journal left behind by an interrupted native transaction.
    ensure_apps_closed()?;
    let mut db = writer(&source)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("library is busy")?;
    for (i, grid) in journal.files.iter().enumerate() {
        for (name, expected) in [("original", grid.before), ("reviewed", grid.after)] {
            let file = directory.join(format!("{i}.{name}"));
            regular(&file)?;
            ensure!(
                sha256_file(&file)? == expected,
                "recovery grid checksum mismatch"
            );
        }
    }
    integrity(&tx)?;
    let state = logical_digest(&tx)?;
    ensure!(state == journal.before_db || state == journal.after_db, "native database has newer or unrecognized changes; no recovery files were applied; journal: {}", directory.display());
    ensure_apps_closed()?;
    copy_grids(&journal, &directory, state == journal.after_db)?;
    fs::remove_file(marker)?;
    sync_directory(&source)?;
    tx.commit()?;
    Ok(finish_recovery_directory(directory, &journal))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: Option<tempfile::TempDir>,
        source: PathBuf,
        backups: PathBuf,
        relative: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let root = tempfile::tempdir()?;
            let base = root.path().canonicalize()?;
            let source = base.join("desktop");
            let relative: PathBuf = "share/PIONEER/USBANLZ/ANLZ0000.DAT".into();
            fs::create_dir_all(source.join(&relative).parent().unwrap())?;
            fs::write(source.join(&relative), b"original beats")?;
            let db = native(
                &source.join("master.db"),
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            )?;
            for (table, cols) in crate::rbimport::REQUIRED_SCHEMA {
                db.execute(&format!("CREATE TABLE {table} ({})", cols.join(", ")), [])?;
            }
            db.execute_batch("INSERT INTO djmdContent(ID,Rating) VALUES('track',1)")?;
            Ok(Self {
                _root: Some(root),
                source,
                backups: base.join("backups"),
                relative,
            })
        }
        fn db(&self) -> Result<Connection> {
            native(
                &self.source.join("master.db"),
                OpenFlags::SQLITE_OPEN_READ_WRITE,
            )
        }
        fn bytes(&self) -> Result<Vec<u8>> {
            Ok(fs::read(self.source.join(&self.relative))?)
        }
        fn change(&self) -> FileChange {
            FileChange {
                relative_path: self.relative.clone(),
                expected_sha256: Sha256::digest(b"original beats").into(),
                contents: b"reviewed beats".to_vec(),
            }
        }
        // Simulate either side of a crash at the SQLite commit boundary, with
        // the ANLZ rename either already visible or still at its old contents.
        fn interrupted(&self, committed: bool, published: bool, crash: bool) -> Result<PathBuf> {
            self.interrupted_with_policy(committed, published, crash, RecoveryCopy::Keep)
        }
        fn interrupted_with_policy(&self, committed: bool, published: bool, crash: bool, policy: RecoveryCopy) -> Result<PathBuf> {
            let mut db = self.db()?;
            db.pragma_update(None, "cache_size", 4)?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let directory = self.backups.join(if policy == RecoveryCopy::Keep { "fixture" } else { "recordbox-transaction-fixture" });
            fs::create_dir_all(&directory)?;
            let before = if policy == RecoveryCopy::Keep {
                backup_database(&self.source, &directory)?;
                None
            } else { Some(logical_digest(&tx)?) };
            tx.execute("UPDATE djmdContent SET rating=3", [])?;
            if crash {
                tx.execute("UPDATE spill SET chunk=zeroblob(8192)", [])?;
            }
            let journal = stage_grids(&self.source, &directory, &tx, vec![self.change()], before)?;
            let manifest = serde_json::to_vec(&journal)?;
            save_new(&directory.join(MANIFEST), &manifest)?;
            let marker = Pending {
                directory: directory.clone(),
                manifest_sha256: Sha256::digest(&manifest).into(),
            };
            save_new(&self.source.join(MARKER), &serde_json::to_vec(&marker)?)?;
            if published {
                copy_grids(&journal, &directory, true)?;
            }
            if crash {
                std::process::exit(83);
            }
            if committed {
                tx.commit()?;
            }
            Ok(directory)
        }
    }

    #[test]
    fn recognizes_desktop_process_names_without_matching_recordbox() {
        for name in [
            "rekordbox.exe",
            "C:\\Pioneer\\rekordboxAgent.exe",
            "/Applications/rekordbox 7.app/Contents/MacOS/rekordbox",
            "rekordbox Agent",
            "rekordbox Helper (GPU)",
        ] {
            assert!(app_process(name), "{name}");
        }
        for other in [
            "Recordbox",
            "recordbox.exe",
            "rekordbox-migration-helper",
            "shell script referencing rekordbox",
        ] {
            assert!(!app_process(other), "{other}");
        }
    }

    #[test]
    fn os_process_list_detects_a_running_named_executable() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let image = if cfg!(windows) {
            "rb-wbprobe.exe"
        } else {
            "rb-wbprobe"
        };
        let exe = folder.path().join(image);
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            let system = std::env::var_os("SystemRoot").context("missing Windows system folder")?;
            fs::copy(PathBuf::from(system).join("System32/ping.exe"), &exe)?;
            Command::new(&exe)
                .args(["-n", "5", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .creation_flags(0x08000000)
                .spawn()?
        };
        #[cfg(unix)]
        let mut child = {
            fs::copy("/bin/sleep", &exe)?;
            Command::new(&exe).arg("10").spawn()?
        };
        let outcome = check_process_list(|p| p.trim().rsplit(['/', '\\']).next() == Some(image));
        let _ = child.kill();
        child.wait()?;
        let err = outcome.expect_err("OS process enumeration missed a running executable");
        assert!(err.to_string().contains("close rekordbox"), "{err}");
        Ok(())
    }

    #[test]
    fn independent_recordbox_lock_and_rollback_on_callback_error() -> Result<()> {
        let fixture = Fixture::new()?;
        let other = operation_lock(&fixture.source, false)?;
        assert!(operation_lock(&fixture.source, false).is_err());
        drop(other);
        let db = fixture.db()?;
        db.execute_batch("BEGIN IMMEDIATE")?;
        assert!(apply::<()>(&fixture.source, &fixture.backups, |_| panic!(
            "competing writer must be refused"
        ))
        .is_err());
        db.execute_batch("ROLLBACK")?;
        assert!(apply::<()>(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=5", [])?;
            bail!("simulated failure")
        })
        .is_err());
        assert_eq!(
            db.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert!(!pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn native_table_triggers_require_separate_validation() -> Result<()> {
        let fixture = Fixture::new()?;
        let db = fixture.db()?;
        db.execute_batch("CREATE TRIGGER secondary_changes AFTER UPDATE ON djmdContent BEGIN UPDATE djmdContent SET Rating=0; END;")?;
        let error = apply::<()>(&fixture.source, &fixture.backups, |_| {
            panic!("unreviewed trigger must stop all writes")
        })
        .unwrap_err();
        assert!(error.to_string().contains("secondary_changes"));
        assert_eq!(
            db.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert!(!fixture.backups.exists());
        Ok(())
    }

    #[test]
    fn backups_include_wal_and_grid_replacements() -> Result<()> {
        let fixture = Fixture::new()?;
        let other = fixture.db()?;
        other.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA wal_checkpoint(TRUNCATE); UPDATE djmdContent SET Rating=2")?;
        assert!(fs::metadata(fixture.source.join("master.db-wal"))?.len() > 0);
        let ((), backup) = apply(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=4", [])?;
            Ok(((), vec![fixture.change()]))
        })?;
        let backup_db = native(&backup.join("master.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        assert_eq!(
            backup_db.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            other.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            4
        );
        assert_eq!(fs::read(backup.join("0.original"))?, b"original beats");
        assert_eq!(fixture.bytes()?, b"reviewed beats");
        assert!(!pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn optional_database_backup_uses_sql_rollback_and_refuses_unjournaled_grids() -> Result<()> {
        let f = Fixture::new()?;
        let update = |db: &Connection| -> Result<()> { db.execute("UPDATE djmdContent SET Rating=5", [])?; Ok(()) };
        assert!(apply_with_policy::<()>(&f.source, &f.backups, RecoveryCopy::DatabaseTransaction, |db| {
            update(db)?; bail!("interrupted before commit")
        }).is_err());
        assert!(apply_with_policy(&f.source, &f.backups, RecoveryCopy::DatabaseTransaction, |db| {
            update(db)?; Ok(((), vec![f.change()]))
        }).is_err());
        assert_eq!(f.db()?.query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?, 1);
        assert!(!pending(&f.source) && !f.backups.exists());
        assert_eq!(f.bytes()?, b"original beats");
        Ok(())
    }

    #[test]
    fn optional_grid_backup_retires_only_its_own_files_after_success_or_recovery() -> Result<()> {
        let f = Fixture::new()?;
        let (_, retained) = apply_with_policy(&f.source, &f.backups, RecoveryCopy::TemporaryForGrids, |db| {
            db.execute("UPDATE djmdContent SET Rating=5", [])?;
            Ok(((), vec![f.change()]))
        })?;
        assert!(retained.backup.is_none() && retained.remaining_staging.is_none());
        assert_eq!(fs::read_dir(&f.backups)?.count(), 0);
        assert_eq!(f.bytes()?, b"reviewed beats");
        assert!(!pending(&f.source));

        for (committed, added_file) in [(false, false), (true, false), (false, true)] {
            let f = Fixture::new()?;
            let directory = f.interrupted_with_policy(committed, !committed, false, RecoveryCopy::TemporaryForGrids)?;
            assert!(!directory.join("master.db").exists() && !directory.join("backup.json").exists());
            if added_file { fs::write(directory.join("my-file"), b"keep")?; }
            let retained = recover(&f.source, &f.backups)?;
            if added_file {
                assert!(retained.backup.is_none());
                assert_eq!(retained.remaining_staging.as_deref(), Some(directory.as_path()));
                assert_eq!(fs::read(directory.join("my-file"))?, b"keep");
                assert!(directory.join("0.original").is_file());
            } else {
                assert!(retained.backup.is_none() && retained.remaining_staging.is_none() && !directory.exists());
            }
            assert!(!pending(&f.source));
            assert_eq!(f.bytes()?, if committed { &b"reviewed beats"[..] } else { &b"original beats"[..] });
        }
        Ok(())
    }

    #[test]
    fn staging_only_recovery_refuses_unrecognized_database_sidecars_and_manifest() -> Result<()> {
        for target in ["database", "original", "manifest"] {
            let f = Fixture::new()?;
            let directory = f.interrupted_with_policy(false, true, false, RecoveryCopy::TemporaryForGrids)?;
            match target {
                "database" => { f.db()?.execute("INSERT INTO djmdContent(ID,Rating) VALUES('later',5)", [])?; }
                "original" => fs::write(directory.join("0.original"), b"user replaced the staging file")?,
                _ => {
                    let mut manifest = read_bounded(&directory.join(MANIFEST), MAX_MANIFEST)?;
                    manifest.push(b' ');
                    fs::write(directory.join(MANIFEST), manifest)?;
                }
            }
            assert!(recover(&f.source, &f.backups).is_err());
            assert!(pending(&f.source) && directory.is_dir());
            assert_eq!(f.bytes()?, b"reviewed beats");
        }
        Ok(())
    }

    #[test]
    fn interrupted_grids_follow_the_state_that_committed() -> Result<()> {
        for (committed, installed) in [(false, true), (true, false), (true, true)] {
            let fixture = Fixture::new()?;
            let backup = fixture.interrupted(committed, installed, false)?;
            assert!(apply::<()>(&fixture.source, &fixture.backups, |_| panic!(
                "pending transaction must stop new writes"
            ))
            .is_err());
            assert_eq!(recover(&fixture.source, &fixture.backups)?.backup, Some(backup));
            let expected: &[u8] = if committed {
                b"reviewed beats"
            } else {
                b"original beats"
            };
            assert_eq!(fixture.bytes()?, expected);
            assert!(!pending(&fixture.source));
        }
        Ok(())
    }

    #[test]
    fn process_exit_with_a_hot_rollback_journal_restores_matching_grids() -> Result<()> {
        const CHILD_FIXTURE: &str = "RECORDBOX_TEST_HOT_JOURNAL_FIXTURE";
        if let Some(folder) = std::env::var_os(CHILD_FIXTURE) {
            let base = PathBuf::from(folder);
            let child = Fixture {
                _root: None,
                source: base.join("desktop"),
                backups: base.join("backups"),
                relative: "share/PIONEER/USBANLZ/ANLZ0000.DAT".into(),
            };
            let policy = if std::env::var_os("RECORDBOX_TEST_SKIP_BACKUP") == Some("1".into()) {
                RecoveryCopy::TemporaryForGrids
            } else { RecoveryCopy::Keep };
            child.interrupted_with_policy(false, true, true, policy)?;
            bail!("child should have exited with the transaction still open");
        }
        for temporary in [false, true] {
            let fixture = Fixture::new()?;
            fixture.db()?.execute_batch("CREATE TABLE spill(chunk BLOB); WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<40) INSERT INTO spill SELECT randomblob(8192) FROM n;")?;
            let module = module_path!().split_once("::").unwrap().1;
            let test =
                format!("{module}::process_exit_with_a_hot_rollback_journal_restores_matching_grids");
            let child = Command::new(std::env::current_exe()?)
                .args(["--exact", &test])
                .env(CHILD_FIXTURE, fixture.source.parent().unwrap())
                .env("RECORDBOX_TEST_SKIP_BACKUP", if temporary { "1" } else { "0" })
                .output()?;
            assert_eq!(
                child.status.code(),
                Some(83),
                "{} {}",
                String::from_utf8_lossy(&child.stdout),
                String::from_utf8_lossy(&child.stderr)
            );
            assert!(fs::metadata(fixture.source.join("master.db-journal"))?.len() > 0);
            assert!(pending(&fixture.source));
            assert_eq!(fixture.bytes()?, b"reviewed beats");
            let files = recover(&fixture.source, &fixture.backups)?;
            assert_eq!(files.backup.is_none(), temporary);
            assert!(files.remaining_staging.is_none());
            assert_eq!(fixture.bytes()?, b"original beats");
            assert_eq!(
                fixture
                    .db()?
                    .query_row("SELECT rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
                1
            );
        }
        Ok(())
    }

    #[test]
    fn recovery_preserves_subsequent_untouched_work() -> Result<()> {
        let fixture = Fixture::new()?;
        fixture.interrupted(false, true, false)?;
        fs::write(fixture.source.join(&fixture.relative), b"later file")?;
        assert!(recover(&fixture.source, &fixture.backups).is_err());
        assert_eq!(fixture.bytes()?, b"later file");
        assert!(pending(&fixture.source));
        fs::write(fixture.source.join(&fixture.relative), b"reviewed beats")?;
        fixture
            .db()?
            .execute("INSERT INTO djmdContent(ID,Rating) VALUES('later',5)", [])?;
        assert!(recover(&fixture.source, &fixture.backups).is_err());
        assert_eq!(fixture.bytes()?, b"reviewed beats");
        assert!(pending(&fixture.source));
        Ok(())
    }

    #[test]
    fn refuses_traversal_and_changed_grid_before_committing_sql() -> Result<()> {
        let fixture = Fixture::new()?;
        for path in ["../ANLZ0000.DAT", "share/../master.db", "master.db"] {
            assert!(grid_target(&fixture.source, Path::new(path)).is_err());
        }
        let proposed = fixture.change();
        fs::write(fixture.source.join(&fixture.relative), b"an external edit")?;
        assert!(apply(&fixture.source, &fixture.backups, |db| {
            db.execute("UPDATE djmdContent SET Rating=4", [])?;
            Ok(((), vec![proposed]))
        })
        .is_err());
        assert_eq!(fixture.bytes()?, b"an external edit");
        assert_eq!(
            fixture
                .db()?
                .query_row("SELECT Rating FROM djmdContent", [], |r| r.get::<_, i64>(0))?,
            1
        );
        assert!(!pending(&fixture.source));
        Ok(())
    }
}
