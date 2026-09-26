//! The data directory (`REMOTEHUB_CONNECTOR_DATA`): access state, journal,
//! users and the web interface's certificate.

use std::io;
use std::path::Path;

/// Creates the data directory, readable only by the connector's user.
pub fn data_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Writes a file only the connector's user may read: password hashes, TOTP
/// secrets, the certificate's key. On Windows, the data directory's access
/// list protects it (#166).
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?
            .write_all(contents)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}
