use crate::prelude::*;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

// guaranteed to be relative, normalized (by being a Vec), valid filenames across
// Windows/macOS/Linux, valid utf8. We don't currently rule out all the Windows device
// names though (CON, LPT, etc.).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum NicePathComponent {
    Parent,
    Normal(String),
}

// https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
const NAUGHTY_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

impl NicePathComponent {
    pub fn try_from_bytes(value: &[u8]) -> Result<NicePathComponent> {
        NicePathComponent::try_from_str(std::str::from_utf8(value)?)
    }

    pub fn try_from_str(value: &str) -> Result<NicePathComponent> {
        if value.is_empty() {
            bail!("path components must be non-empty");
        }
        if value.contains(&*NAUGHTY_CHARS) {
            bail!("invalid or non-portable characters in path component {value:?}");
        }
        if value.contains(|c: char| c.is_ascii_control()) {
            bail!("invalid or non-portable characters in path component {value:?}");
        }
        if value.ends_with('.') || value.ends_with(' ') {
            bail!("invalid or non-portable path component {value:?}");
        }
        Ok(NicePathComponent::Normal(value.into()))
    }
}

impl Display for NicePathComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NicePathComponent::Parent => write!(f, ".."),
            NicePathComponent::Normal(piece) => write!(f, "{}", piece),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NicePathBuf(Vec<NicePathComponent>);

impl NicePathBuf {
    // This is the low-level private constructor, which doesn't check for
    // "contained-ness" -- it can freely return paths that start with '../'
    //
    // It's called by the public constructors (TryFrom<...> for NicePathBuf), and also
    // the NiceSymlinkPaths constructor (because symlink targets *are* allowed to start
    // with ..).
    fn from_unix(value: &typed_path::UnixPath) -> Result<NicePathBuf> {
        use typed_path::unix::UnixComponent::*;
        let mut new = NicePathBuf(Default::default());
        for c in value.components() {
            match c {
                RootDir => bail!("expected relative path"),
                CurDir => (),
                ParentDir => {
                    if let Some(NicePathComponent::Normal(_)) = new.0.last() {
                        new.0.pop();
                    } else {
                        new.0.push(NicePathComponent::Parent);
                    }
                }
                Normal(piece) => {
                    new.0.push(NicePathComponent::try_from_bytes(piece)?);
                }
            }
        }
        Ok(new)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn components(&self) -> impl Iterator<Item = &NicePathComponent> {
        self.0.iter()
    }

    pub fn to_native(&self) -> PathBuf {
        self.into()
    }
}

impl Display for NicePathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.len() == 0 {
            write!(f, ".")
        } else {
            let pieces = self.components().map(|c| c.to_string()).collect::<Vec<_>>();
            write!(f, "{}", pieces.as_slice().join("/"))
        }
    }
}

impl TryFrom<&typed_path::UnixPath> for NicePathBuf {
    type Error = anyhow::Error;

    #[context("validating path {}", value.display())]
    fn try_from(value: &typed_path::UnixPath) -> Result<Self, Self::Error> {
        let new = NicePathBuf::from_unix(value)?;
        if new.0.first() == Some(&NicePathComponent::Parent) {
            bail!("path escapes containment");
        }
        Ok(new)
    }
}

impl TryFrom<&str> for NicePathBuf {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.as_bytes().try_into()
    }
}

impl TryFrom<&[u8]> for NicePathBuf {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        typed_path::UnixPath::new(value).try_into()
    }
}

impl From<&NicePathBuf> for PathBuf {
    fn from(value: &NicePathBuf) -> Self {
        if value.len() == 0 {
            PathBuf::from(".")
        } else {
            let mut new = PathBuf::new();
            for c in value.components() {
                match c {
                    NicePathComponent::Parent => new.push(".."),
                    NicePathComponent::Normal(piece) => new.push(piece),
                }
            }
            new
        }
    }
}

#[derive(Debug)]
pub struct NiceSymlinkPaths {
    pub source: NicePathBuf,
    pub target: NicePathBuf,
}

impl NiceSymlinkPaths {
    pub fn new(source: &NicePathBuf, target_bytes: &[u8]) -> Result<NiceSymlinkPaths> {
        let target_unix = typed_path::UnixPath::new(target_bytes);
        let target = NicePathBuf::from_unix(target_unix)?;
        let target_dotdots = target
            .0
            .iter()
            .filter(|c| *c == &NicePathComponent::Parent)
            .count();
        // Example: symlink foo/bar -> ../../outside
        //
        //   foo/bar -> 2 entries
        //   ../../outside -> 2 ..'s
        //   2 < 1 + 2 -> fail (which is correct)
        //
        // Conceptually: we always go up one path segment b/c symlinks are resolved
        // against the source's directory, not the source itself (e.g. foo/bar -> baz
        // resolves to foo/baz, not foo/bar/baz).
        if source.len() < 1 + target_dotdots {
            bail!("symlink {} -> {} escapes confinement", source, target);
        }
        Ok(NiceSymlinkPaths { source: source.clone(), target })
    }
}

pub trait WriteTree {
    fn mkdir(&mut self, path: &NicePathBuf) -> Result<()>;
    fn write_file(
        &mut self,
        path: &NicePathBuf,
        data: &mut dyn Read,
        executable: bool,
    ) -> Result<()>;
    fn write_symlink(&mut self, symlink: &NiceSymlinkPaths) -> Result<()>;
}

pub struct WriteTreeFS {
    root: PathBuf,
}

impl WriteTreeFS {
    pub fn new<T: AsRef<Path>>(root: T) -> WriteTreeFS {
        WriteTreeFS {
            root: root.as_ref().into(),
        }
    }
}

impl WriteTree for WriteTreeFS {
    fn mkdir(&mut self, path: &NicePathBuf) -> Result<()> {
        Ok(fs::create_dir(self.root.join(path.to_native()))?)
    }

    fn write_file(
        &mut self,
        path: &NicePathBuf,
        data: &mut dyn Read,
        executable: bool,
    ) -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if executable {
            options.mode(0o777);
        } else {
            options.mode(0o666);
        }
        let mut file = options.open(self.root.join(path.to_native()))?;
        io::copy(data, &mut file)?;
        Ok(())
    }

    fn write_symlink(&mut self, symlink: &NiceSymlinkPaths) -> Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                symlink.target.to_native(),
                symlink.source.to_native(),
            )?;
        }
        #[cfg(not(unix))]
        {
            bail!("symlinks not supported on this platform");
        }
        Ok(())
    }
}

pub fn unpack_zip_carefully<T: Read + Seek, W: WriteTree>(
    z: &mut ZipArchive<T>,
    mut dir: W,
) -> Result<()> {
    // we process symlinks in a batch at the end
    let mut symlinks = Vec::<NiceSymlinkPaths>::new();
    for i in 0..z.len() {
        let mut zip_file = z.by_index(i)?;
        if let Some(mode) = zip_file.unix_mode() {
            if mode & 0xf000 == 0xa000 {
                // it's a symlink
                symlinks.push(NiceSymlinkPaths::new(
                    &zip_file.name().try_into()?,
                    slurp(&mut zip_file)?.as_slice(),
                )?);
                continue;
            }
        }
        let path: NicePathBuf = zip_file.name().try_into()?;
        if zip_file.is_dir() {
            dir.mkdir(&path)?;
        } else {
            let executable = zip_file
                .unix_mode()
                .map(|v| v & 0o0111 != 0)
                .unwrap_or(false);
            dir.write_file(&path, &mut zip_file, executable)?;
        }
    }

    // process symlinks in order from longest to shortest, to prevent weird cases where
    // first we make a symlink foo/ -> bar/, and then we make another symlink foo/baz ->
    // something.
    symlinks.sort_unstable_by_key(|symlink| symlink.source.len());
    for symlink in symlinks.into_iter().rev() {
        dir.write_symlink(&symlink)?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_nice_path_buf() {
        for bad in [
            "foo/../../bar",
            "../foo",
            "/nope",
            "c:\\bad",
            "not\\good",
            "what\x00",
        ] {
            assert!(TryInto::<NicePathBuf>::try_into(bad).is_err());
        }

        for (input, normed) in [
            ("foo/bar/baz/", "foo/bar/baz"),
            ("foo/.././//baz", "baz"),
            (".///.", "."),
        ] {
            assert_eq!(
                TryInto::<NicePathBuf>::try_into(input).unwrap().to_string(),
                normed.to_string()
            );
        }
    }

    #[test]
    fn test_s() {
        for (source, target) in [
            ("foo", ".."),
            ("foo/bar", "../../more/segments/here"),
            ("foo/bar/", "../../nope"),
            ("foo", "/etc/shadow"),
        ] {
            println!("{source} -> {target}");
            assert!(NiceSymlinkPaths::new(
                &source.try_into().unwrap(),
                target.as_bytes()
            )
            .is_err());
        }
        for (source, target, normalized) in [
            ("foo/bar", "..", ".."),
            ("foo", "./baz/bar", "baz/bar"),
            ("foo/bar/baz", "something/../../..//./stuff/../thing", "../../thing"),
        ] {
            println!("{source} -> {target}");
            let symlink = NiceSymlinkPaths::new(&source.try_into().unwrap(),
                                      target.as_bytes()).unwrap();
            assert_eq!(symlink.target.to_string(), normalized.to_string());
        }
    }

    // XX TODO: write some tests that unpacking invalid zip files are rejected!!
}
