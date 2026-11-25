use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn is_executable(path: &Path) -> bool {
    path.metadata().is_ok_and(|meta| {
        #[cfg(unix)]
        if meta.permissions().mode() & 0o111 == 0 {
            return false;
        }
        meta.is_file()
    })
}

pub fn find(name: &OsStr) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.file_name() != Some(name) {
        return is_executable(path)
            .then(|| std::path::absolute(path).ok())
            .flatten();
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(path))
        .find(|candidate| is_executable(candidate))
        .and_then(|candidate| std::path::absolute(candidate).ok())
}
