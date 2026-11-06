use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

pub(crate) fn open(data_dir: &Path, purpose: &str) -> io::Result<File> {
    let parent = data_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let name = data_dir
        .file_name()
        .ok_or_else(|| io::Error::other("missing data directory name"))?;
    let resolved = match data_dir.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => parent.canonicalize()?.join(name),
        Err(error) => return Err(error),
    };
    let parent = resolved
        .parent()
        .ok_or_else(|| io::Error::other("missing data directory parent"))?;
    let name = resolved
        .file_name()
        .ok_or_else(|| io::Error::other("missing data directory name"))?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(format!(".{}.{purpose}.lock", name.to_string_lossy())))
}
