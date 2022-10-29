use crate::prelude::*;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::SeekFrom;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use ring::digest;

// A simple on-disk key-value store for static blobs of data. Used for stuff like
// caches, holding a forest of unpacked wheels within a directory without conflicts,
// etc.
//
// Keys are anything whose reference type implements PathKey. In practice this is mostly
// ArtifactHash, which produces a nicely formed path, or an arbitrary &[u8] blob, which
// gets hashed to produce some arbitrary fixed-length path. Both use urlsafe-base64.
//
// For each key, we have a lockfile, + a file of data.
//
// Updating the data file is done by writing the new data into a temporary file and then
// renaming it into place, so it ought to be atomic. For this rename, we use
// NamedTempFile::persist.
//
// On Unix, this is trivially atomic. On Windows, tempfile v3.3.0 uses MoveFileExW. And
// according to Doug Cook here:
//
//   https://social.msdn.microsoft.com/Forums/windowsdesktop/en-US/449bb49d-8acc-48dc-a46f-0760ceddbfc3/movefileexmovefilereplaceexisting-ntfs-same-volume-atomic
//
// ...this should basically always be atomic when the source and destination
// locations are both within the same directory. If it ever becomes a problem, we
// can also try using the newer SetFileInformationByHandle's posix rename option:
//
//   https://stackoverflow.com/a/51737582/
//
// The locking has two purposes:
//
// - It prevents "dogpiling", where multiple independent instances of this program waste
//   energy on filling in the same entry at the same time.
// - In the future, we hope it will allow us to safely do garbage collection on the
//   cache, because it will let the GC process safely assume that no-one is using the
//   entry it's about to delete.
//
// We do bend the rules on locking in one case: we allow read-only file descriptors
// pointing into a cache file to escape without holding the lock. This is safe on the
// assumption that:
//
// - Cache files are never modified, only deleted or replaced.
// - Our OS allows open files to be deleted/replaced, and lets existing file descriptors
//   continue to access the deleted file.
//
// Again, on Unix this is trivial. On Windows, this is the semantics starting in Windows
// 10, assuming you open the file with FILE_SHARE_DELETE (which is the default for
// Rust):
//
//   https://github.com/golang/go/issues/32088#issuecomment-502850674
//
// On Windows 7, it's probably not true? The old Windows behavior was that
// FILE_SHARE_DELETE meant you were allowed to request that the file be deleted, but it
// wouldn't actually happen until all handles were closed. This means currently, our
// atomic updates are currently broken on Windows 7. If this becomes a problem, the
// workaround is to rename the old file out of the way before renaming the new file into
// place, and then deleting the old file under its new name. But Win7 is EOL, so,
// whatever.

// thoughts on adding GC:
// - when accessing a key should update the mtime on the lock file; that's an easy way
//   to keep track of what's most recently used
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
    let enc = data_encoding::BASE64URL_NOPAD.encode(&bytes);
    for i in 0..DIR_NEST_DEPTH {
        path.push(&enc[i..i + 1]);
    }
    path.push(&enc[DIR_NEST_DEPTH..]);
    path
}

pub trait PathKey {
    fn key(&self) -> PathBuf;
}

impl PathKey for &[u8] {
    fn key(&self) -> PathBuf {
        let scrambled_key = digest::digest(&digest::SHA256, self);
        bytes_to_path_suffix(scrambled_key.as_ref())
    }
}

impl PathKey for &ArtifactHash {
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
    open_options.append(true);
    match mode {
        LockMode::Lock => {
            let dir = lock_path.parent().unwrap();
            fs::create_dir_all(dir).with_context(|| {
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
    lock.lock_exclusive()?;
    Ok(lock)
}

#[derive(Debug)]
pub struct KVDir {
    base: PathBuf,
}

impl KVDir {
    pub fn new(base: &Path) -> KVDir {
        KVDir { base: base.into() }
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
            let mut writer = handle.begin()?;
            f(&mut writer)?;
            Ok(Box::new(writer.commit()?.detach_unlocked()))
        }
    }

    pub fn get_contents_if_exists<K: PathKey>(
        &self,
        key: &K,
    ) -> Option<Box<dyn ReadPlusSeek>> {
        if let Some(handle) = self.lock_if_exists(key) {
            if let Some(reader) = handle.reader() {
                return Some(Box::new(reader.detach_unlocked()));
            }
        }
        return None;
    }

    pub fn lock<T: PathKey>(&self, key: &T) -> Result<KVDirLock> {
        let path = self.base.join(key.key());
        let lock = lock(&path, LockMode::Lock)?;
        Ok(KVDirLock { _lock: lock, path })
    }

    // the reason this exists is to make it possible to probe for cache entries without
    // creating tons of directories/lock files that will never be used.
    pub fn lock_if_exists<T: PathKey>(&self, key: &T) -> Option<KVDirLock> {
        let path = self.base.join(key.key());
        if let Ok(lock) = lock(&path, LockMode::IfExists) {
            Some(KVDirLock { _lock: lock, path })
        } else {
            None
        }
    }
}

pub struct KVDirLock {
    _lock: File,
    path: PathBuf,
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
        let mut f = self.f.persist(&self.path)?;
        f.rewind()?;
        Ok(LockedRead {
            f,
            _lifetime: self._lifetime,
        })
    }
}

impl KVDirLock {
    pub fn reader<'a>(&'a self) -> Option<LockedRead<'a>> {
        Some(LockedRead {
            f: File::open(&self.path).ok()?,
            _lifetime: Default::default(),
        })
    }

    pub fn begin<'a>(&'a self) -> Result<LockedWrite<'a>> {
        Ok(LockedWrite {
            path: &self.path,
            // unwrap() safe b/c entry paths always have a parent
            f: tempfile::NamedTempFile::new_in(&self.path.parent().unwrap())?,
            _lifetime: Default::default(),
        })
    }
}
