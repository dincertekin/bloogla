//! Databases and backups contain sign-in sessions. Keep them private even
//! when the person running Bloogla has a permissive umask.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub fn protect_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn create_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Missing parents are private from creation; existing unrelated parents
/// (such as /tmp or the user's backup destination) keep their permissions.
pub fn create_parents(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Tighten directories Bloogla owns, including ones from earlier versions.
pub fn create_directory(path: &Path) -> io::Result<()> {
    create_parents(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn protect_database(path: &Path) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let file = Path::new(&name);
        if file.exists() {
            protect_file(file)?;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn existing_owned_directories_are_tightened_without_changing_their_parent() {
        let parent = std::env::temp_dir().join(format!(
            "bloogla-permissions-test-{}",
            crate::app::security::random_hex(8)
        ));
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        let owned = parent.join("data");
        std::fs::create_dir(&owned).unwrap();
        std::fs::set_permissions(&owned, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_directory(&owned).unwrap();
        assert_eq!(
            std::fs::metadata(&owned).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            0o755
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}
