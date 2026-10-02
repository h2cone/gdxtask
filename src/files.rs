//! File publication helpers. Atomicity applies to a single replacement, not
//! a collection of files or a crash-safe multi-file transaction.

#[cfg(windows)]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
use std::{fs, io::Write, path::Path, time::Duration};
#[cfg(windows)]
use std::{thread, time::Instant};
use tempfile::NamedTempFile;

/// Retry bounded Windows sharing/access conflicts; other OS errors return
/// immediately. A zero timeout requests a single attempt.
#[derive(Clone, Copy, Debug)]
pub struct ReplaceOptions {
    pub timeout: Duration,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl Default for ReplaceOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(1),
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
        }
    }
}

impl ReplaceOptions {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.initial_delay.is_zero() && self.max_delay >= self.initial_delay,
            "replacement retry delays must be positive and max_delay >= initial_delay"
        );
        Ok(())
    }
}

/// Stage bytes beside the destination, sync them, and atomically replace the
/// destination. Unique temporary names avoid collisions between writers and
/// are removed on failure. The parent directory is not fsynced.
pub fn atomic_write(path: &Path, bytes: &[u8], options: &ReplaceOptions) -> Result<()> {
    options.validate()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut staged = NamedTempFile::new_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    // Close the temporary file before replacement to avoid retaining our own
    // open handle. TempPath still cleans up on both failure and success.
    let staged = staged.into_temp_path();
    replace_file(&staged, path, options)
}

pub fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// Replace a file using caller-owned staging on the same filesystem.
#[cfg(not(windows))]
pub fn replace_file(source: &Path, destination: &Path, options: &ReplaceOptions) -> Result<()> {
    options.validate()?;
    fs::rename(source, destination).with_context(|| {
        format!(
            "replace {} with {}",
            destination.display(),
            source.display()
        )
    })
}

/// Replace a file using caller-owned staging on the same filesystem.
#[cfg(windows)]
pub fn replace_file(source: &Path, destination: &Path, options: &ReplaceOptions) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    options.validate()?;

    let wide_path = |path: &Path| -> Result<Vec<u16>> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        ensure!(
            !wide.contains(&0),
            "file replacement path contains a NUL byte"
        );
        wide.push(0);
        Ok(wide)
    };
    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    let started = Instant::now();
    let mut retry_delay = options.initial_delay;
    loop {
        // SAFETY: Both buffers are NUL-terminated and remain alive for the call.
        let replaced = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced != 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        let transient_lock = matches!(
            error.raw_os_error(),
            Some(code)
                if code == ERROR_ACCESS_DENIED as i32
                    || code == ERROR_SHARING_VIOLATION as i32
        );
        if !transient_lock || started.elapsed() >= options.timeout {
            bail!("atomic file replacement failed: {error}");
        }
        thread::sleep(retry_delay.min(options.timeout.saturating_sub(started.elapsed())));
        retry_delay = retry_delay.saturating_mul(2).min(options.max_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn publication_replaces_existing_files_and_cleans_failed_staging() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("output");
        atomic_write(&target, b"old", &ReplaceOptions::default()).unwrap();
        atomic_write(&target, b"new", &ReplaceOptions::default()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        let directory = temp.path().join("directory");
        fs::create_dir(&directory).unwrap();
        assert!(atomic_write(&directory, b"failure", &ReplaceOptions::default()).is_err());
        assert!(directory.is_dir());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn embedded_nul_does_not_truncate_native_replacement_paths() {
        use std::{
            ffi::OsString,
            os::windows::ffi::{OsStrExt, OsStringExt},
        };
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::write(&source, b"new").unwrap();
        fs::write(&target, b"old").unwrap();
        let mut invalid = target.as_os_str().encode_wide().collect::<Vec<_>>();
        invalid.extend([0, 120]);
        let invalid = std::path::PathBuf::from(OsString::from_wide(&invalid));
        assert!(replace_file(&source, &invalid, &ReplaceOptions::default()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(source.exists());
    }

    #[cfg(windows)]
    #[test]
    fn transient_reader_lock_is_retried_and_timeout_preserves_old_contents() {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("manifest.json");
        fs::write(&target, b"old").unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&target)
            .unwrap();
        let once = ReplaceOptions {
            timeout: Duration::ZERO,
            ..Default::default()
        };
        assert!(atomic_write(&target, b"new", &once).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            drop(reader);
        });
        atomic_write(&target, b"new", &ReplaceOptions::default()).unwrap();
        release.join().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }
}
