//! Private file and directory helpers (owner-only permissions on Unix).

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// Creates `dir` and missing parents. On Unix, directories created here get
/// mode 0700; existing directories are left as they are.
pub(crate) fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn private_options() -> OpenOptions {
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// Creates the file if missing (mode 0600 on Unix) without touching content.
pub(crate) fn ensure_private_file(path: &Path) -> io::Result<()> {
    private_options()
        .create(true)
        .append(true)
        .open(path)
        .map(drop)
}

/// Creates or truncates a file for writing (mode 0600 on Unix).
pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    private_options()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
}
