//! Shared construction for the runtime modules.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use neotales_exec::{Command, find};

/// Resolves `name` through the registry, falling back to the bare name.
pub(crate) fn command(name: &str, args: Vec<OsString>) -> Command {
    let program = find(name, None).unwrap_or_else(|_| PathBuf::from(name));
    Command::new(program).args(args)
}

/// Applies the file-versus-inline rule: one line ending in a known extension is a file.
pub(crate) fn script_file<'a>(text: &'a str, extensions: &[&str]) -> Option<&'a str> {
    if text.contains('\n') || text.contains('\r') {
        return None;
    }

    let trimmed = text.trim();
    if extensions
        .iter()
        .any(|extension| trimmed.ends_with(extension))
    {
        Some(trimmed)
    } else {
        None
    }
}

/// Writes a fresh private script file and leaves it on disk.
///
/// On Windows privacy relies on the per-user `LocalAppData` directory and reparse-point refusal,
/// not a custom ACL.
pub(crate) fn temp_file(script: &str, extension: &str) -> io::Result<PathBuf> {
    let directory = private_directory()?;
    let result = write_script_with(&directory, extension, |file| {
        file.write_all(script.as_bytes())
    });
    if result.is_err() {
        let _ = std::fs::remove_dir(&directory);
    }
    result
}

fn random_name() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(io::Error::other)?;
    let mut name = String::with_capacity(32);
    for byte in bytes {
        write!(name, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Ok(name)
}

fn private_directory() -> io::Result<PathBuf> {
    let base = private_base()?;
    loop {
        let path = base.join(format!("neotales-exec-script-{}", random_name()?));
        match create_directory(&path) {
            Ok(()) => {
                #[cfg(unix)]
                let verified = std::fs::File::open(&path)
                    .and_then(|file| file.metadata())
                    .and_then(|metadata| verify_unix_directory(&metadata, true));
                #[cfg(windows)]
                let verified = refuse_reparse_points(&path);
                if let Err(error) = verified {
                    let _ = std::fs::remove_dir(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn create_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(not(unix))]
fn create_directory(path: &Path) -> io::Result<()> {
    DirBuilder::new().create(path)
}

fn environment_directory(key: &str) -> io::Result<PathBuf> {
    let path = std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::other("user directory is missing or not absolute"))?;
    Ok(path)
}

#[cfg(unix)]
fn verify_unix_directory(metadata: &std::fs::Metadata, private: bool) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mode = metadata.mode() & 0o7777;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || if private {
            mode != 0o700
        } else {
            mode & 0o022 != 0
        }
    {
        return Err(io::Error::other(
            "user directory is not private or owned by the current user",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match create_directory(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    verify_unix_directory(&std::fs::symlink_metadata(path)?, true)
}

#[cfg(unix)]
fn private_base() -> io::Result<PathBuf> {
    let home = environment_directory("HOME")?;
    private_base_in(&home)
}

#[cfg(unix)]
fn private_base_in(home: &Path) -> io::Result<PathBuf> {
    use std::path::Component;

    if !home.is_absolute() || home.components().any(|part| part == Component::ParentDir) {
        return Err(io::Error::other(
            "home directory must be an absolute path without parent components",
        ));
    }
    // Rebuild from components so a trailing slash cannot cause symlink_metadata to follow a link.
    let home: PathBuf = home.components().collect();
    verify_unix_directory(&std::fs::symlink_metadata(&home)?, false)?;
    for ancestor in home.ancestors().skip(1) {
        verify_ancestor(&std::fs::symlink_metadata(ancestor)?)?;
    }
    let base = home.join(".neotales-exec-script");
    ensure_private_directory(&base)?;
    Ok(base)
}

#[cfg(unix)]
fn trusted_ancestor(owner: u32, mode: u32, current: u32) -> bool {
    if owner == 0 {
        mode & 0o022 == 0 || mode & 0o1000 != 0
    } else {
        owner == current && mode & 0o022 == 0
    }
}

#[cfg(unix)]
fn verify_ancestor(metadata: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if !metadata.is_dir()
        || !trusted_ancestor(
            metadata.uid(),
            metadata.mode(),
            rustix::process::getuid().as_raw(),
        )
    {
        return Err(io::Error::other("home directory has an untrusted ancestor"));
    }
    Ok(())
}

#[cfg(windows)]
fn private_base() -> io::Result<PathBuf> {
    let local = environment_directory("LOCALAPPDATA")?;
    refuse_reparse_points(&local)?;
    let base = local.join("neotales-exec-script");
    match create_directory(&base) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    refuse_reparse_points(&base)?;
    if !std::fs::symlink_metadata(&base)?.is_dir() {
        return Err(io::Error::other("user directory is not a directory"));
    }
    Ok(base)
}

#[cfg(windows)]
fn refuse_reparse_points(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x0400;
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor)?.file_attributes() & REPARSE_POINT != 0 {
            return Err(io::Error::other(
                "a temporary directory path contains a reparse point",
            ));
        }
    }
    Ok(())
}

fn write_script_with(
    directory: &Path,
    extension: &str,
    write_body: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let path = directory.join(format!("run-{}.{extension}", random_name()?));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let result = write_body(&mut file).and_then(|()| file.flush());
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_failure_leaves_no_file_behind() {
        let directory = private_directory().expect("create directory");
        let result = write_script_with(&directory, "go", |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("the disk is full"))
        });
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_dir(&directory)
                .expect("read directory")
                .count(),
            0
        );
        std::fs::remove_dir(&directory).expect("remove directory");
    }

    #[cfg(unix)]
    #[test]
    fn home_permissions_and_unrelated_cache() {
        use std::os::unix::fs::PermissionsExt;
        let home = private_directory().expect("create stand-in home");
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o720)).unwrap();
        assert!(private_base_in(&home).is_err());
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cache = home.join(".cache");
        std::os::unix::fs::symlink("missing", &cache).unwrap();
        assert_eq!(
            private_base_in(&home).unwrap(),
            home.join(".neotales-exec-script")
        );
        std::fs::remove_file(&cache).unwrap();
        create_directory(&cache).unwrap();
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert_eq!(
            private_base_in(&home).unwrap(),
            home.join(".neotales-exec-script")
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ancestors_require_trusted_owners_and_permissions() {
        assert!(trusted_ancestor(0, 0o755, 1000));
        assert!(trusted_ancestor(0, 0o1777, 1000));
        assert!(!trusted_ancestor(0, 0o777, 1000));
        assert!(trusted_ancestor(1000, 0o755, 1000));
        assert!(!trusted_ancestor(1000, 0o1777, 1000));
        assert!(!trusted_ancestor(1001, 0o700, 1000));
    }

    #[cfg(unix)]
    #[test]
    fn home_rejects_writable_and_symlinked_ancestors() {
        use std::os::unix::fs::PermissionsExt;
        let root = private_directory().unwrap();
        let parent = root.join("parent");
        let home = parent.join("home");
        create_directory(&parent).unwrap();
        create_directory(&home).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(private_base_in(&home).is_err());
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&parent, &link).unwrap();
        assert!(private_base_in(&link.join("home")).is_err());
        assert!(private_base_in(Path::new(&format!("{}/", link.display()))).is_err());
        assert!(private_base_in(&link.join("home/..")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
