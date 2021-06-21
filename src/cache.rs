use crate::prelude::*;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};

use ring::digest;

// Some filesystems don't cope well with a single directory containing lots of files. So
// we disperse our files over multiple nested directories. This is the nesting depth, so
// "3" means our paths will look like:
//   ${BASE}/${CHAR}/${CHAR}/${CHAR}/${ENTRY}
// And our fanout is 64, so this would split our files over 64**3 = 262144 directories.
const DIR_NEST_DEPTH: usize = 3;

/// Naive key/value cache, one value per file.
///
/// Should be mostly robust against concurrent updates, though I'm not quite sure how
/// well it will handle things on windows. Guess we'll see!
///
/// Currently has no eviction policy at all -- just grows without bound. That's... bad,
/// maybe? Does pip's cache have any eviction policy? Do we need to store any extra
/// metadata to help with evictions (e.g. last accessed time)?
#[derive(Debug, Clone)]
pub struct Cache {
    base: PathBuf,
}

#[derive(Debug, Copy, Clone)]
pub enum Basket {
    // url -> (page contents + etag to allow revalidation)
    Etagged,
    // url -> wheel/sdist/whatever
    Artifact,
    // url -> METADATA file
    PackageMetadata,
    // XX todo for locally-built wheels. What should this be indexed by? sdist url?
    //LocallyBuilt,
}

impl Basket {
    fn dirname(&self) -> &str {
        match self {
            Basket::Etagged => "etagged",
            Basket::Artifact => "artifacts",
            Basket::PackageMetadata => "metadata",
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            base: PROJECT_DIRS.cache_dir().into(),
        }
    }
}

impl Cache {
    pub fn new(base: &Path) -> Cache {
        Cache {
            base: base.to_path_buf(),
        }
    }

    fn path_for_key(&self, basket: Basket, key: &str) -> PathBuf {
        let scrambled_key = data_encoding::BASE64URL_NOPAD
            .encode(digest::digest(&digest::SHA256, key.as_bytes()).as_ref());
        let mut path = self.base.clone();
        path.push(basket.dirname());
        for i in 0..DIR_NEST_DEPTH {
            path.push(&scrambled_key[i..i + 1]);
        }
        path.push(&scrambled_key[DIR_NEST_DEPTH..]);
        path
    }

    pub fn get_file(&self, basket: Basket, key: &str) -> Option<File> {
        let p = self.path_for_key(basket, key);
        File::open(p).ok()
    }

    pub fn get(&self, basket: Basket, key: &str) -> Option<Vec<u8>> {
        match self.get_file(basket, &key) {
            None => None,
            Some(mut f) => {
                let mut buf = Vec::new();
                match f.read_to_end(&mut buf) {
                    Err(_) => None,
                    Ok(_) => Some(buf),
                }
            }
        }
    }

    // XX this is mostly for saving large artifacts without loading them into memory...
    // probably want to keep the file open and return it? and also hash it? do we want a
    // callback to do the writing or what?
    pub fn put_file<T>(&self, basket: Basket, key: &str, write: T) -> Result<File>
    where
        T: FnOnce(&mut File) -> Result<()>,
    {
        let p = self.path_for_key(basket, key);
        fs::create_dir_all(p.parent().unwrap())
            .context("Failed to create cache directory")?;
        let mut tmp = tempfile::NamedTempFile::new_in(&self.base)?;

        write(&mut tmp.as_file_mut())?;
        let mut f = tmp.persist(&p)?;
        f.seek(SeekFrom::Start(0))?;
        Ok(f)
    }

    pub fn put(&self, basket: Basket, key: &str, data: &[u8]) -> Result<()> {
        self.put_file(basket, &key, |f| Ok(f.write_all(data)?))?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn tmp_cache() -> (tempfile::TempDir, Cache) {
        let d = tempfile::TempDir::new().unwrap();
        let c = Cache::new(d.path());
        (d, c)
    }

    #[test]
    fn test_cache_basic() {
        let (_d, c) = tmp_cache();

        // Can save and restore
        c.put(Basket::Etagged, "foo", b"foo value").unwrap();
        assert_eq!(c.get(Basket::Etagged, "foo"), Some(b"foo value".to_vec()));

        // Can overwrite values
        c.put(Basket::Etagged, "foo", b"new value").unwrap();
        assert_eq!(c.get(Basket::Etagged, "foo"), Some(b"new value".to_vec()));

        // Different baskets have separate keyspaces
        c.put(Basket::Artifact, "foo", b"other value").unwrap();
        assert_eq!(
            c.get(Basket::Artifact, "foo"),
            Some(b"other value".to_vec())
        );
        assert_eq!(c.get(Basket::Etagged, "foo"), Some(b"new value".to_vec()));
    }

    #[test]
    fn test_cache_streaming() {
        let (_d, c) = tmp_cache();

        let mut f = c
            .put_file(Basket::PackageMetadata, "key", |f| Ok(f.write_all(b"xxx")?))
            .unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"xxx".to_vec());

        assert_eq!(
            c.get(Basket::PackageMetadata, "key").unwrap(),
            b"xxx".to_vec()
        );

        let mut f2 = c.get_file(Basket::PackageMetadata, "key").unwrap();
        let mut buf2 = Vec::new();
        f2.read_to_end(&mut buf2).unwrap();
        assert_eq!(buf2, b"xxx".to_vec());
    }
}
