//! Space available to the current user on the filesystem containing a directory.

use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Space {
    pub capacity: u64,
    pub available: u64,
}

pub(crate) fn on(path: &Path) -> io::Result<Space> {
    let space = query(path)?;
    if space.capacity == 0 || space.available > space.capacity {
        return Err(io::Error::other("the filesystem reported inconsistent capacity or available space"));
    }
    Ok(space)
}

#[cfg(unix)]
fn query(path: &Path) -> io::Result<Space> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    // The path is NUL-terminated; a successful call initializes the output structure.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let fragment = if stat.f_frsize == 0 { stat.f_bsize } else { stat.f_frsize };
    let bytes = |blocks| u64::try_from(u128::from(fragment) * blocks)
        .map_err(|_| io::Error::other("filesystem byte count exceeds supported range"));
    Ok(Space { capacity: bytes(u128::from(stat.f_blocks))?, available: bytes(u128::from(stat.f_bavail))? })
}

#[cfg(windows)]
fn query(path: &Path) -> io::Result<Space> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in volume path")); }
    // UNC shares require the trailing separator. It also works for local directories.
    if !name.ends_with(&[u16::from(b'\\')]) && !name.ends_with(&[u16::from(b'/')]) { name.push(u16::from(b'\\')); }
    name.push(0);
    let mut space = Space { capacity: 0, available: 0 };
    // The first two outputs honor per-user disk quotas; total free bytes can include
    // space this process is not allowed to allocate.
    if unsafe { GetDiskFreeSpaceExW(name.as_ptr(), &mut space.available, &mut space.capacity, std::ptr::null_mut()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(space)
}

#[cfg(not(any(unix, windows)))]
fn query(_: &Path) -> io::Result<Space> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "filesystem capacity unavailable"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_volume_containing_a_nested_directory() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("cache with spaces");
        std::fs::create_dir(&child).unwrap();
        let space = super::on(&child).unwrap();
        assert!(space.capacity > 0 && space.available <= space.capacity);
        assert_eq!(space.capacity, super::on(dir.path()).unwrap().capacity);
        assert!(super::on(&child.join("absent")).is_err());
    }
}
