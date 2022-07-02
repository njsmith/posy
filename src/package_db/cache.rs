use crate::prelude::*;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::SeekFrom;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use fs2::FileExt;

// Some filesystems don't cope well with a single directory containing lots of files. So
// we disperse our files over multiple nested directories. This is the nesting depth, so
// "3" means our paths will look like:
//   ${BASE}/${CHAR}/${CHAR}/${CHAR}/${ENTRY}
// And our fanout is 64, so this would split our files over 64**3 = 262144 directories.
const DIR_NEST_DEPTH: usize = 3;

// We rely heavily on NamedTempFile::persist being atomic, to prevent cache corruption.
//
// On Unix this is trivial. On Windows, tempfile v3.3.0 uses MoveFileExW. And according
// to Doug Cook here:
//
//   https://social.msdn.microsoft.com/Forums/windowsdesktop/en-US/449bb49d-8acc-48dc-a46f-0760ceddbfc3/movefileexmovefilereplaceexisting-ntfs-same-volume-atomic
//
// ...this should basically always be atomic when the source and destination
// locations are both within the same directory. If it ever becomes a problem, we
// can also try using the newer SetFileInformationByHandle's posix rename option:
//
//   https://stackoverflow.com/a/51737582/
fn atomic_replace_with<F, T>(path: &Path, thunk: &F) -> Result<File>
    // needs to return Result so this can detect failure and delete instead of
    // persisting
    where F: FnOnce(&mut dyn Write) -> Result<T>
{
    let dir = path.parent().unwrap();
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    let value = thunk(&mut tmp)?;
    let mut f = tmp.persist(&path)?;
    f.rewind()?;
    Ok(f)
}


fn bytes_to_path_suffix(bytes: &[u8]) -> PathBuf {
    let mut path = PathBuf::new();
    let enc = data_encoding::BASE64URL_NOPAD.encode(&bytes);
    for i in 0..DIR_NEST_DEPTH {
        path.push(&enc[i..i + 1]);
    }
    path.push(&enc[DIR_NEST_DEPTH..]);
    path
}

fn path_for_hash(base: &Path, hash: &ArtifactHash) -> PathBuf {
    let mut path = base.to_path_buf();
    path.push(&hash.mode);
    path.push(bytes_to_path_suffix(&hash.raw_data));
    path
}

fn lock(path: &Path) -> Result<File> {
    let mut lock_path = path.to_path_buf();
    // unwrap rationale: this function should never be passed paths with trailing /
    let mut basename = lock_path.file_name().unwrap().to_os_string();
    basename.push(".lock");
    lock_path.set_file_name(basename);
    fs::create_dir_all(lock_path.parent().unwrap())
        .context("Failed to create cache directory")?;
    let lock = fs::OpenOptions::new().append(true).create(true).open(lock_path)?;
    lock.lock_exclusive()?;
    Ok(lock)
}

#[derive(Debug)]
pub struct ImmutableFileCache {
    base: PathBuf,
}

impl ImmutableFileCache {
    // This only needs locking to avoid "dogpile" issues when multiple instances are
    // trying to fetch the same artifact at the same time. Once the file is known to be
    // on disk, it's safe to return an fd pointing to it even without holding the lock.
    pub fn get_or_fetch<F>(&self, key: &ArtifactHash, fetch: &F) -> Result<File>
        where F: FnOnce(&mut dyn Write) -> Result<()>
    {
        let path = path_for_hash(&self.base, key);
        let lock = lock(&path)?;
        match File::open(path) {
            Ok(file) => Ok(file),
            Err(_) => {
                let f = atomic_replace_with(&path, fetch)?;
                Ok(f)
            }
        }
    }
}

#[derive(Debug)]
pub struct CacheDir {
    base: PathBuf,
}

impl CacheDir {
    pub fn get(&self, key: &[u8]) -> Result<CacheHandle> {
        let path = self.base.join(bytes_to_path_suffix(key));
        let lock = lock(&path)?;
        Ok(CacheHandle { lock, path })
    }
}

pub struct CacheHandle {
    lock: File,
    path: PathBuf,
}

struct LockedRead<'a> {
    f: File,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Read for LockedRead<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.f.read(&mut buf)
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

struct LockedWrite<'a> {
    path: &'a Path,
    f: tempfile::NamedTempFile,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> Write for LockedWrite<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.f.write(&mut buf)
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
        let mut f= self.f.persist(&self.path)?;
        f.rewind()?;
        Ok(LockedRead { f, _lifetime: self._lifetime })
    }
}

impl CacheHandle {
    pub fn reader<'a>(&'a self) -> Option<impl 'a + Read + Seek> {
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

/// Currently has no eviction policy at all -- just grows without bound. That's... bad,
/// maybe? Does pip's cache have any eviction policy? Do we need to store any extra
/// metadata to help with evictions (e.g. last accessed time)?
#[derive(Debug)]
pub struct PackageCache {
    pub index_pages: CacheDir,
    pub artifacts: ImmutableFileCache,
    pub metadata: ImmutableFileCache,
    // todo: cache mapping sdist hash -> directory containing build wheels
    //locally_built_wheels: ImmutableFilesInMutableDirectoryCache
    // todo: and some sort of caching for direct url references? gotta think about how
    // that would work. probably just a generic HTTP cache, fold in index_pages?
    // ...should we use that for artifact downloads too?
    // and same problem for direct VCS references, though that will probably have a
    // bunch of domain-specific complexities. (maybe only cache wheel, under the VCS
    // revision hash?)
}

// maybe switch the artifact-from-index cache to use plain http caching too? for pypi
// certainly it works well. and I should probably make my pybi index give proper
// cache-control too.
// advantage is that it removes the whole issue with figuring out which hash we want
// etc.
// probably would want to store as a directory with:
// - cache metadata
// - body
// - hash values that we've verified for the body?
//   (or maybe sha256 is fast enough now we don't care about caching this)

impl PackageCache {
    pub fn new(base: &Path) -> PackageCache {
        PackageCache {
            index_pages: CacheDir { base: base.join("index-pages") },
            artifacts: ImmutableFileCache { base: base.join("artifacts") },
            metadata: ImmutableFileCache { base: base.join("metadata") },
        }
    }
}
