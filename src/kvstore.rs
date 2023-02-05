use crate::prelude::*;
use crate::util::retry_interrupted;
use auto_impl::auto_impl;
use fs2::FileExt;
use ring::digest;
use std::fs::{self, File};
use std::io::SeekFrom;
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};

// A simple on-disk key-value store for static blobs of data. Each key maps to a
// different path on disk. Used for stuff like caches, holding a forest of unpacked
// wheels within a directory without conflicts, etc.
//
// Keys are anything whose reference type implements PathKey. In practice this is mostly
// ArtifactHash, which produces a nicely formed path, or an arbitrary &[u8] blob, which
// gets hashed to produce some arbitrary fixed-length path. Both use urlsafe-base64.
//
// For each key, we have a lockfile, + a file or directory of data.
//
// For KVFileStore:
//
//   Updating the data file is done by writing the new data into a temporary file and
//   then renaming it into place, so it ought to be atomic. For this rename, we use
//   NamedTempFile::persist.
//
//   On Unix, this is trivially atomic. On Windows, tempfile v3.3.0 uses MoveFileExW. And
//   according to Doug Cook here:
//
//     https://social.msdn.microsoft.com/Forums/windowsdesktop/en-US/449bb49d-8acc-48dc-a46f-0760ceddbfc3/movefileexmovefilereplaceexisting-ntfs-same-volume-atomic
//
//   ...this should basically always be atomic when the source and destination
//   locations are both within the same directory. If it ever becomes a problem, we
//   can also try using the newer SetFileInformationByHandle's posix rename option:
//
//     https://stackoverflow.com/a/51737582/
//
// For KVDirStore:
//
//   Each entry "value" is an arbitrary directory. We still use rename to make writes
//   mostly-atomic, mostly to avoid corruption in the case of crashes, but since
//   directories don't support atomic-replace and you can't keep a handle on a deleted
//   directory, we have to be more careful with concurrent access.
//
// For both types of stores, we use a lock file to manage access to each key. For right
// now, this is a simple exclusive lock, that we take during lookup/mutation, and then
// drop after the lookup/mutation is complete -- but we continue to access the (file
// descriptor / directory path) after dropping the lock.
//
// For KVFileStore, this is pretty harmless, because if someone does come along later to
// mutate the value, they replace the underlying file, so any existing fds remain valid.
//
//   [Reference: On Unix this is trivial. On Windows, this is the semantics starting in
//   Windows 10, assuming you open the file with FILE_SHARE_DELETE, which is the default
//   for Rust:
//
//      https://github.com/golang/go/issues/32088#issuecomment-502850674
//
//   On Windows 7, it's probably not true? The old Windows behavior was that
//   FILE_SHARE_DELETE meant you were allowed to request that the file be deleted, but
//   it wouldn't actually happen until all handles were closed. This means currently,
//   our atomic updates are currently broken on Windows 7. If this becomes a problem,
//   the workaround is to rename the old file out of the way before renaming the new
//   file into place, and then deleting the old file under its new name. But Win7 is
//   EOL, so, whatever.]
//
// For KVDirStore, that's not the case, so for now only implement "write once, read
// many" semantics, and we'll extend as necessary.
//
// The locking is useful though to prevent races on writing to the same key, and
// avoiding dogpiling (where multiple independent instances of this program waste energy
// on computing+writing the same entry at the same time).
//
// In the future, I want to add some kind of GC support, for pruning caches and clearing
// out old no-longer-used wheels. This will require more complex locking strategies,
// though, so leaving that an XX TODO for now.

// thoughts on adding GC:
// - when accessing a key should update the mtime on the lock file; that's an easy way
//   to keep track of what's most recently used for cache cleanup
// - for cleaning things up... can scan everything and for old files, take the
//   lock and then delete the payload? but then how do we clean up the lockfile and
//   directories themselves? I guess we don't have to but accumulating an unbounded
//   collection of empty inodes seems a bit rude.
//   maybe better: have a global lockfile at the root of the cache, which we normally
//   acquire in shared mode. use its mtime to track when the last time GC ran was.
//   opportunistically try to acquire it in exclusive mode; if succeed can run GC.
//   otherwise acquire in shared mode and let someone else worry about GC later.

// Some filesystems don't cope well with a single directory containing lots of files. So
// we disperse our files over multiple nested directories. This is the nesting depth, so
// "3" means our paths will look like:
//   ${BASE}/${CHAR}/${CHAR}/${CHAR}/${ENTRY}
// And our fanout is 64, so this would split our files over 64**3 = 262144 directories.
const DIR_NEST_DEPTH: usize = 3;

fn bytes_to_path_suffix(bytes: &[u8]) -> PathBuf {
    let mut path = PathBuf::new();
    let enc = data_encoding::BASE64URL_NOPAD.encode(bytes);
    for i in 0..DIR_NEST_DEPTH {
        path.push(&enc[i..i + 1]);
    }
    path.push(&enc[DIR_NEST_DEPTH..]);
    path
}

#[auto_impl(&)]
pub trait PathKey {
    fn key(&self) -> PathBuf;
}

impl PathKey for [u8] {
    fn key(&self) -> PathBuf {
        let scrambled_key = digest::digest(&digest::SHA256, self);
        bytes_to_path_suffix(scrambled_key.as_ref())
    }
}

impl PathKey for ArtifactHash {
    fn key(&self) -> PathBuf {
        let mut path = PathBuf::new();
        path.push(&self.mode);
        path.push(bytes_to_path_suffix(&self.raw_data));
        path
    }
}

enum LockMode {
    Lock,
    IfExists,
}

fn lock(path: &Path, mode: LockMode) -> Result<File> {
    let mut lock_path = path.to_path_buf();
    // unwrap rationale: this function should never be passed paths with trailing /
    let mut basename = lock_path.file_name().unwrap().to_os_string();
    basename.push(".lock");
    lock_path.set_file_name(basename);
    let mut open_options = fs::OpenOptions::new();
    // On Windows, the lock file must be opened in write mode -- append mode isn't good
    // enough.
    open_options.write(true);
    match mode {
        LockMode::Lock => {
            let dir = lock_path.parent().unwrap();
            fs::create_dir_all(dir).wrap_err_with(|| {
                format!("Failed to create directory {}", dir.display())
            })?;
            open_options.create(true);
        }
        LockMode::IfExists => {
            // don't create directory or set create() flag; if it doesn't exist the open
            // will error out.
        }
    };
    let lock = open_options.open(&lock_path)?;
    // fs2::FileExit::lock_exclusive on Unix is a thin wrapper around flock(2), and in
    // particular doesn't handle EINTR.
    retry_interrupted(|| lock.lock_exclusive())?;
    Ok(lock)
}

#[derive(Debug)]
pub struct KVFileStore {
    base: PathBuf,
    tmp: PathBuf,
}

impl KVFileStore {
    pub fn new(base: &Path) -> Result<KVFileStore> {
        let base = std::env::current_dir()?.join(base);
        let tmp = base.join("tmp");
        fs::create_dir_all(&base)?;
        fs::create_dir_all(&tmp)?;
        Ok(KVFileStore {
            base,
            tmp,
        })
    }

    pub fn get_or_set<K: PathKey, F>(
        &self,
        key: &K,
        f: F,
    ) -> Result<Box<dyn ReadPlusSeek>>
    where
        F: FnOnce(&mut dyn Write) -> Result<()>,
    {
        let handle = self.lock(key)?;
        if let Some(reader) = handle.reader() {
            Ok(Box::new(reader.detach_unlocked()))
        } else {
            // XX TODO: on error, call handle.remove (need a custom drop)
            let mut writer = handle.begin()?;
            f(&mut writer)?;
            Ok(Box::new(writer.commit()?.detach_unlocked()))
        }
    }

    pub fn get<K: PathKey>(&self, key: &K) -> Option<Box<dyn ReadPlusSeek>> {
        if let Some(handle) = self.lock_if_exists(key) {
            if let Some(reader) = handle.reader() {
                return Some(Box::new(reader.detach_unlocked()));
            }
        }
        None
    }

    pub fn lock<K: PathKey>(&self, key: &K) -> Result<KVFileLock> {
        let path = self.base.join(key.key());
        let lock = lock(&path, LockMode::Lock)?;
        Ok(KVFileLock {
            tmp: self.tmp.clone(),
            _lock: lock,
            path,
        })
    }

    // the reason this exists is to make it possible to probe for cache entries without
    // creating tons of directories/lock files that will never be used.
    pub fn lock_if_exists<K: PathKey>(&self, key: &K) -> Option<KVFileLock> {
        let path = self.base.join(key.key());
        if let Ok(lock) = lock(&path, LockMode::IfExists) {
            Some(KVFileLock {
                tmp: self.tmp.clone(),
                _lock: lock,
                path,
            })
        } else {
            None
        }
    }
}

pub struct KVFileLock {
    tmp: PathBuf,
    _lock: File,
    path: PathBuf,
}

impl KVFileLock {
    pub fn reader<'a>(&self) -> Option<LockedRead<'a>> {
        Some(LockedRead {
            f: File::open(&self.path).ok()?,
            _lifetime: Default::default(),
        })
    }

    pub fn begin(&self) -> Result<LockedWrite> {
        Ok(LockedWrite {
            path: &self.path,
            f: tempfile::NamedTempFile::new_in(&self.tmp)?,
            _lifetime: Default::default(),
        })
    }

    // XX TODO: also walk up self.path.parent.ancestors() to KVFileStore.base, calling
    // rmdir() on each entry, until one fails, to clean up unnecessary directories?
    // (can't remove the .lock file, because another process might be waiting on that
    // file handle...unless we add a retry loop to lock(), that checks whether the lock
    // file we got matches the one on disk before returning? might be necessary for any
    // kind of GC anyway)
    pub fn remove(self) -> Result<()> {
        fs::remove_file(self.path)?;
        Ok(())
    }
}

pub struct LockedRead<'a> {
    f: File,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Read for LockedRead<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.f.read(buf)
    }
}

impl<'a> Seek for LockedRead<'a> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.f.seek(pos)
    }
}

impl<'a> LockedRead<'a> {
    pub fn detach_unlocked(self) -> File {
        self.f
    }
}

pub struct LockedWrite<'a> {
    path: &'a Path,
    f: tempfile::NamedTempFile,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Write for LockedWrite<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.f.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.f.flush()
    }
}

impl<'a> Seek for LockedWrite<'a> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.f.seek(pos)
    }
}

impl<'a> LockedWrite<'a> {
    pub fn commit(self) -> Result<LockedRead<'a>> {
        self.f.as_file().sync_data()?;
        let mut f = self.f.persist(self.path)?;
        f.rewind()?;
        Ok(LockedRead {
            f,
            _lifetime: self._lifetime,
        })
    }
}

////////////////////////////////////////////////////////////////

pub struct KVDirStore {
    base: PathBuf,
    tmp: PathBuf,
}

impl KVDirStore {
    pub fn new(base: &Path) -> Result<KVDirStore> {
        let base = std::env::current_dir()?.join(base);
        let tmp = base.join("tmp");
        fs::create_dir_all(&base)?;
        fs::create_dir_all(&tmp)?;
        Ok(KVDirStore {
            base,
            tmp,
        })
    }

    pub fn lock<K: PathKey>(&self, key: &K) -> Result<KVDirLock> {
        let path = self.base.join(key.key());
        let lock = lock(&path, LockMode::Lock)?;
        Ok(KVDirLock {
            tmp: self.tmp.clone(),
            _lock: lock,
            path,
        })
    }

    pub fn get_or_set<K, F>(&self, key: &K, f: F) -> Result<PathBuf>
    where
        K: PathKey,
        F: FnOnce(&Path) -> Result<()>,
    {
        let lock = self.lock(&key)?;
        if !lock.exists() {
            let tmp = lock.tempdir()?;
            f(tmp.as_ref())?;
            fs::rename(&tmp.into_path(), &*lock)?;
        }
        Ok(lock.path)
    }
}

pub struct KVDirLock {
    tmp: PathBuf,
    _lock: File,
    path: PathBuf,
}

impl KVDirLock {
    pub fn tempdir(&self) -> Result<tempfile::TempDir> {
        Ok(tempfile::tempdir_in(&self.tmp)?)
    }
}

impl Deref for KVDirLock {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path.deref()
    }
}

impl AsRef<Path> for KVDirLock {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

// XX TODO: seriously need some tests that validate the locking etc.
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_kvfilestore_basics() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = KVFileStore::new(tmp.path())?;

        let hi = b"hi".as_slice();
        let bye = b"bye".as_slice();

        assert_eq!(
            slurp(
                &mut store
                    .get_or_set(&hi, |w| {
                        w.write_all(b"hello")?;
                        Ok(())
                    })
                    .unwrap()
            )
            .unwrap(),
            b"hello",
        );

        assert_eq!(
            slurp(
                &mut store
                    .get_or_set(&hi, |w| {
                        // this never executes because the key already exists
                        w.write_all(b"ASDFASDFSADFSADF")?;
                        Ok(())
                    })
                    .unwrap()
            )
            .unwrap(),
            b"hello",
        );

        assert_eq!(slurp(&mut store.get(&hi).unwrap())?, b"hello");
        assert!(store.get(&bye).is_none());

        assert!(store.lock_if_exists(&bye).is_none());
        let hi_handle = store.lock_if_exists(&hi).unwrap();
        assert_eq!(slurp(&mut hi_handle.reader().unwrap())?, b"hello");
        assert_eq!(
            slurp(&mut hi_handle.reader().unwrap().detach_unlocked())?,
            b"hello"
        );

        let bye_handle = store.lock(&bye)?;
        assert!(bye_handle.reader().is_none());

        let mut w = bye_handle.begin()?;
        w.write_all(b"Good")?;
        w.write_all(b"bye")?;
        let mut r = w.commit()?;
        assert_eq!(slurp(&mut r)?, b"Goodbye");

        Ok(())
    }

    #[test]
    #[cfg(not(windows))]
    fn test_kvfilestore_overwrite() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = KVFileStore::new(tmp.path())?;

        let key = b"my key".as_slice();

        {
            let handle = store.lock(&key)?;
            let mut w = handle.begin()?;
            w.write_all(b"gen 1")?;
            w.commit()?;
        }

        {
            let handle = store.lock(&key)?;
            assert_eq!(slurp(&mut handle.reader().unwrap())?, b"gen 1");
            let mut w = handle.begin()?;
            w.write_all(b"gen 2")?;
            w.commit()?;
        }

        assert_eq!(slurp(&mut store.get(&key).unwrap())?, b"gen 2");

        // And rewrite with the old file still open
        // TODO: this part is broken on Windows!
        let mut old_file = {
            let handle = store.lock(&key)?;
            handle.reader().unwrap().detach_unlocked()
        };
        let new_handle = store.lock(&key)?;
        let mut w = new_handle.begin()?;
        w.write_all(b"gen 3")?;
        let mut r = w.commit()?;

        assert_eq!(slurp(&mut r)?, b"gen 3");
        // old file still has old data
        assert_eq!(slurp(&mut old_file)?, b"gen 2");

        Ok(())
    }

    #[test]
    fn test_kvdirstore_basics() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = KVDirStore::new(tmp.path())?;

        let hi = b"hi".as_slice();

        let path = store.get_or_set(&hi, |t| {
            fs::write(t.join("file"), b"hello")?;
            Ok(())
        })?;

        assert_eq!(fs::read(path.join("file"))?, b"hello");

        Ok(())
    }
}
