//! Installation-local encryption material, kept outside the database and user containers.
use rand::RngCore;
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Write},
    path::Path,
};
use uuid::Uuid;

const KEY_FILE: &str = "repository-token.key";

pub(super) async fn load<F, Fut>(directory: &Path, has_credentials: F) -> Result<[u8; 32], ()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<bool, ()>>,
{
    match read(directory)? {
        Some(key) => Ok(key),
        None if !has_credentials().await? => create(directory),
        None => Err(()),
    }
}

/// Called while the installation lock is held. Existing invalid keys never get replaced.
pub(super) fn read(directory: &Path) -> Result<Option<[u8; 32]>, ()> {
    check_directory(directory)?;
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = match options.open(directory.join(KEY_FILE)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let metadata = file.metadata().map_err(|_| ())?;
    check_file(directory, &metadata)?;
    let mut bytes = Vec::new();
    file.take(33).read_to_end(&mut bytes).map_err(|_| ())?;
    let key: [u8; 32] = bytes.try_into().map_err(|_| ())?;
    Ok(Some(key))
}

/// Publish a complete, synced key without ever replacing an existing path.
pub(super) fn create(directory: &Path) -> Result<[u8; 32], ()> {
    check_directory(directory)?;
    let mut key = [0; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut key).map_err(|_| ())?;
    let temporary = directory.join(format!(".repository-token-{}.tmp", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    private_mode(&mut options);
    let mut file = options.open(&temporary).map_err(|_| ())?;
    let result = publish(directory, &temporary, &mut file, &key);
    drop(file);
    // This exact temporary name belongs to this call; never remove the final key.
    if let Err(error) = fs::remove_file(&temporary)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(());
    }
    result?;
    sync_directory(directory)?;
    read(directory)?.ok_or(())
}

fn publish(directory: &Path, temporary: &Path, file: &mut File, key: &[u8; 32]) -> Result<(), ()> {
    file.write_all(key)
        .and_then(|()| file.sync_all())
        .map_err(|_| ())?;
    match fs::hard_link(temporary, directory.join(KEY_FILE)) {
        Ok(()) => Ok(()),
        // Another creator may have published while this call was in progress.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(()),
    }
}

fn check_directory(path: &Path) -> Result<(), ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o022 != 0 {
            return Err(());
        }
    }
    Ok(())
}

fn check_file(_directory: &Path, metadata: &fs::Metadata) -> Result<(), ()> {
    if !metadata.is_file() || metadata.len() != 32 {
        return Err(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let parent = fs::symlink_metadata(_directory).map_err(|_| ())?;
        if metadata.uid() != parent.uid() || metadata.mode() & 0o077 != 0 {
            return Err(());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
}
#[cfg(windows)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
}
#[cfg(not(any(unix, windows)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(unix)]
fn private_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}
#[cfg(not(unix))]
fn private_mode(_: &mut OpenOptions) {}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| ())
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<(), ()> {
    Ok(())
}

#[cfg(test)]
mod tests;
