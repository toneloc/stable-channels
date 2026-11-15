//! Files that hold secrets: written whole or not at all, with an exact mode, never through a symlink.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Create `dir` and any missing parents with mode 0700; directories that already exist keep their mode.
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// Write `content` to `path` with `mode`, failing with `AlreadyExists` if anything (a symlink included) is already there.
pub fn write_new(path: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    publish(path, content, mode, Publish::CreateOnly)
}

/// Replace whatever is at `path` with `content` and `mode`; a symlink there is replaced, not written through.
pub fn replace(path: &Path, content: &[u8], mode: u32) -> io::Result<()> {
    publish(path, content, mode, Publish::Replace)
}

#[derive(Clone, Copy, PartialEq)]
enum Publish {
    CreateOnly,
    Replace,
}

/// Temp names tried before giving up; a name is taken only when a crash left a file behind.
const TEMP_NAME_ATTEMPTS: u32 = 16;

/// Counter that makes each temp name in this process unique.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn publish(path: &Path, content: &[u8], mode: u32, how: Publish) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let (temp, file) = create_temp(dir, name)?;
    let result = write_and_publish(file, &temp, path, content, mode, how);
    // A hard link leaves the temp name behind, and a failed step can too.
    if how == Publish::CreateOnly || result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    // The file is already in place; a failed directory sync only weakens durability across a power loss.
    let _ = File::open(dir).and_then(|dir| dir.sync_all());
    Ok(())
}

/// Create a fresh temp file next to the target, skipping names a crashed run left behind (a container restarts with the same PID).
fn create_temp(dir: &Path, name: &std::ffi::OsStr) -> io::Result<(PathBuf, File)> {
    create_temp_counted(dir, name, &NEXT_TEMP)
}

fn create_temp_counted(
    dir: &Path,
    name: &std::ffi::OsStr,
    counter: &AtomicU64,
) -> io::Result<(PathBuf, File)> {
    for _ in 0..TEMP_NAME_ATTEMPTS {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            counter.fetch_add(1, Ordering::Relaxed)
        ));
        let temp = dir.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "no free temporary file name next to {}",
            dir.join(name).display()
        ),
    ))
}

fn write_and_publish(
    mut file: File,
    temp: &Path,
    path: &Path,
    content: &[u8],
    mode: u32,
    how: Publish,
) -> io::Result<()> {
    file.write_all(content)?;
    // Set on the open file, so the umask cannot widen it and a swapped name cannot redirect it.
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    match how {
        // link() never follows or replaces an existing entry, so a file created meanwhile is kept.
        Publish::CreateOnly => fs::hard_link(temp, path),
        // rename() replaces the directory entry itself, so a symlink at `path` is never followed.
        Publish::Replace => fs::rename(temp, path),
    }
}

/// Clear the group and other bits of an open regular file; returns the old mode when it changed.
pub fn make_owner_only(file: &File) -> io::Result<Option<u32>> {
    let metadata = file.metadata()?;
    let mode = metadata.permissions().mode() & 0o777;
    if !metadata.is_file() || mode & 0o077 == 0 {
        return Ok(None);
    }
    file.set_permissions(fs::Permissions::from_mode(mode & 0o700))?;
    Ok(Some(mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn mode_of(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn leftover_temp_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn a_new_secret_file_gets_exactly_its_mode_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api_key");

        write_new(&path, b"secret", 0o600).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"secret");
        assert_eq!(mode_of(&path), 0o600);
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn write_new_never_replaces_or_follows_an_existing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing");
        fs::write(&existing, b"keep").unwrap();
        let target = dir.path().join("target");
        fs::write(&target, b"keep too").unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        let dangling = dir.path().join("dangling");
        symlink(dir.path().join("missing"), &dangling).unwrap();

        for path in [&existing, &link, &dangling] {
            let error = write_new(path, b"new", 0o600).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::AlreadyExists,
                "{}",
                path.display()
            );
        }

        assert_eq!(fs::read(&existing).unwrap(), b"keep");
        assert_eq!(fs::read(&target).unwrap(), b"keep too");
        assert!(!dir.path().join("missing").exists());
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn replace_swaps_out_a_symlink_instead_of_writing_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let protected = dir.path().join("api_key");
        fs::write(&protected, [0x42u8; 32]).unwrap();
        let cert = dir.path().join("tls.crt");
        symlink(&protected, &cert).unwrap();

        replace(&cert, b"certificate", 0o644).unwrap();

        assert_eq!(fs::read(&protected).unwrap(), [0x42u8; 32]);
        assert!(!fs::symlink_metadata(&cert)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&cert).unwrap(), b"certificate");
        assert_eq!(mode_of(&cert), 0o644);
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn a_replace_that_cannot_be_moved_into_place_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tls.crt");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), b"x").unwrap();

        assert!(replace(&path, b"certificate", 0o644).is_err());

        assert!(path.join("inside").exists());
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn new_directories_are_private_and_existing_ones_keep_their_mode() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();

        create_private_dir_all(&existing.join("a").join("b")).unwrap();

        assert_eq!(mode_of(&existing), 0o755);
        assert_eq!(mode_of(&existing.join("a")), 0o700);
        assert_eq!(mode_of(&existing.join("a").join("b")), 0o700);
    }

    #[test]
    fn make_owner_only_clears_group_and_other_bits_and_only_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api_key");
        fs::write(&path, b"key").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();

        assert_eq!(
            make_owner_only(&File::open(&path).unwrap()).unwrap(),
            Some(0o664)
        );
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(make_owner_only(&File::open(&path).unwrap()).unwrap(), None);
        assert_eq!(mode_of(&path), 0o600);
    }

    fn plant_stale_temps(dir: &Path, count: u64) -> Vec<PathBuf> {
        // The names a restarted container (same PID, counter back at 0) would try first.
        (0..count)
            .map(|n| {
                let stale = dir.join(format!(".api_key.{}.{n}.tmp", std::process::id()));
                fs::write(&stale, b"left by a crash").unwrap();
                stale
            })
            .collect()
    }

    #[test]
    fn temp_files_left_by_a_crashed_run_are_skipped_and_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let stale = plant_stale_temps(dir.path(), 8);

        let (temp, _file) =
            create_temp_counted(dir.path(), "api_key".as_ref(), &AtomicU64::new(0)).unwrap();

        assert_eq!(
            temp,
            dir.path()
                .join(format!(".api_key.{}.8.tmp", std::process::id()))
        );
        for file in &stale {
            assert_eq!(fs::read(file).unwrap(), b"left by a crash");
        }
    }

    #[test]
    fn running_out_of_temp_names_fails_without_deleting_anything() {
        let dir = tempfile::tempdir().unwrap();
        let stale = plant_stale_temps(dir.path(), u64::from(TEMP_NAME_ATTEMPTS));

        let error =
            create_temp_counted(dir.path(), "api_key".as_ref(), &AtomicU64::new(0)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        for file in &stale {
            assert_eq!(fs::read(file).unwrap(), b"left by a crash");
        }
    }
}
