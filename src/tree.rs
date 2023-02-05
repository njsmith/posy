use crate::prelude::*;
use auto_impl::auto_impl;
use std::fs;
use std::io;
use std::ops::Deref;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::slice::SliceIndex;
use typed_path::unix::UnixComponent;
use typed_path::UnixPath;
use zip::ZipArchive;

// guaranteed to be relative, contained within the parent directory, normalized (by
// being a Vec), valid filenames across Windows/macOS/Linux, valid utf8. We don't
// currently rule out all the Windows device names though (CON, LPT, etc.).
#[derive(Debug, PartialEq, Eq, Clone, DeserializeFromStr, SerializeDisplay)]
pub struct NicePathBuf {
    pieces: Vec<String>,
}

// https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
const NAUGHTY_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

fn check_path_piece(piece: &[u8]) -> Result<&str> {
    let piece = std::str::from_utf8(piece)?;
    if piece.is_empty() {
        bail!("path components must be non-empty");
    }
    if piece.contains(&*NAUGHTY_CHARS) {
        bail!("invalid or non-portable characters in path component {piece:?}");
    }
    if piece.contains(|c: char| c.is_ascii_control()) {
        bail!("invalid or non-portable characters in path component {piece:?}");
    }
    if piece.ends_with('.') || piece.ends_with(' ') {
        bail!("invalid or non-portable path component {piece:?}");
    }
    Ok(piece)
}

impl NicePathBuf {
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn to_native(&self) -> PathBuf {
        self.into()
    }

    pub fn contains(&self, other: &NicePathBuf) -> bool {
        other.pieces.starts_with(&self.pieces)
    }

    pub fn join(&self, other: &NicePathBuf) -> NicePathBuf {
        let mut pieces = self.pieces.clone();
        for piece in &other.pieces {
            pieces.push(piece.clone());
        }
        NicePathBuf { pieces }
    }

    pub fn pieces(&self) -> &[String] {
        self.pieces.as_slice()
    }

    pub fn slice<I>(&self, index: I) -> NicePathBuf
    where
        I: SliceIndex<[String], Output = [String]>,
    {
        NicePathBuf {
            pieces: self.pieces[index].into(),
        }
    }
}

impl Display for NicePathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pieces.is_empty() {
            write!(f, ".")
        } else {
            write!(f, "{}", self.pieces.as_slice().join("/"))
        }
    }
}

impl TryFrom<&UnixPath> for NicePathBuf {
    type Error = eyre::Report;

    fn try_from(value: &UnixPath) -> Result<Self, Self::Error> {
        context!("validating path {}", value.display());
        let mut new = NicePathBuf { pieces: vec![] };
        for c in value.components() {
            match c {
                UnixComponent::RootDir => bail!("expected relative path"),
                UnixComponent::CurDir => (),
                UnixComponent::ParentDir => {
                    if !new.pieces.is_empty() {
                        new.pieces.pop();
                    } else {
                        bail!("path escapes parent directory");
                    }
                }
                UnixComponent::Normal(piece) => {
                    new.pieces.push(check_path_piece(piece)?.into());
                }
            }
        }
        Ok(new)
    }
}

impl TryFrom<&str> for NicePathBuf {
    type Error = eyre::Report;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.as_bytes().try_into()
    }
}

try_from_str_boilerplate!(NicePathBuf);

impl TryFrom<&[u8]> for NicePathBuf {
    type Error = eyre::Report;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        UnixPath::new(value).try_into()
    }
}

impl From<&NicePathBuf> for PathBuf {
    fn from(value: &NicePathBuf) -> Self {
        value.to_string().into()
    }
}

#[derive(Debug)]
pub struct NiceSymlinkPaths {
    pub source: NicePathBuf,
    pub target: String,
}

impl NiceSymlinkPaths {
    pub fn new(source: &NicePathBuf, target_bytes: &[u8]) -> Result<NiceSymlinkPaths> {
        context!(
            "validating symlink {} -> {}",
            source,
            String::from_utf8_lossy(target_bytes)
        );
        if source.pieces.is_empty() {
            bail!("symlink source can't be '.'");
        }
        let mut sanitized = Vec::<String>::new();
        // We're counting '..'s on the symlink target, because we want to know if it
        // goes up enough to escape the target, when resolved using 'source'. Since
        // symlinks are resolved against the source's parent, they effectively get one
        // '..' "for free".
        let mut dotdots = 1usize;
        for c in UnixPath::new(target_bytes).components() {
            match c {
                UnixComponent::RootDir => {
                    bail!("symlink target must be a relative path")
                }
                UnixComponent::CurDir => (),
                UnixComponent::ParentDir => {
                    match sanitized.last().map(|s| s.as_str()) {
                        None | Some("..") => {
                            sanitized.push("..".into());
                            dotdots = dotdots
                                .checked_add(1)
                                .ok_or(eyre!("too many '..'s"))?;
                        }
                        Some(_) => {
                            sanitized.pop();
                        }
                    }
                }
                UnixComponent::Normal(piece) => {
                    sanitized.push(check_path_piece(piece)?.into());
                }
            }
        }
        if source.len() < dotdots {
            bail!("symlink escapes confinement");
        }
        let target = if sanitized.is_empty() {
            ".".into()
        } else {
            sanitized.as_slice().join("/")
        };
        Ok(NiceSymlinkPaths {
            source: source.clone(),
            target,
        })
    }
}

#[auto_impl(&mut)]
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

    fn full_path(&self, path: &NicePathBuf) -> Result<PathBuf> {
        let full_path = self.root.join(path.to_native());
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(full_path)
    }
}

impl WriteTree for WriteTreeFS {
    fn mkdir(&mut self, path: &NicePathBuf) -> Result<()> {
        context!("Creating {path}/");
        Ok(fs::create_dir(self.full_path(path)?)?)
    }

    fn write_file(
        &mut self,
        path: &NicePathBuf,
        data: &mut dyn Read,
        executable: bool,
    ) -> Result<()> {
        context!("Writing out {path}");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if executable {
            options.mode(0o777);
        } else {
            options.mode(0o666);
        }
        let mut file = options.open(&self.full_path(path)?)?;
        io::copy(data, &mut file)?;
        Ok(())
    }

    fn write_symlink(&mut self, symlink: &NiceSymlinkPaths) -> Result<()> {
        context!("Symlinking {} -> {}", symlink.source, symlink.target);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                &symlink.target,
                &self.full_path(&symlink.source)?,
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
    dest: &mut W,
) -> Result<()> {
    // we process symlinks in a batch at the end
    let mut symlinks = Vec::<NiceSymlinkPaths>::new();
    // indices is sorted from end to start; flip it back around when iterating to get
    // better locality on our reads.
    for i in 0..z.len() {
        let mut zip_file = z.by_index(i)?;
        context!("Unpacking zip file member {}", zip_file.name());
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
            dest.mkdir(&path)?;
        } else {
            let executable = zip_file
                .unix_mode()
                .map(|v| v & 0o0111 != 0)
                .unwrap_or(false);
            dest.write_file(&path, &mut zip_file, executable)?;
        }
    }

    // process symlinks in order from longest to shortest, to prevent weird cases where
    // first we make a symlink foo/ -> bar/, and then we make another symlink foo/baz ->
    // something.
    symlinks.sort_unstable_by_key(|symlink| symlink.source.len());
    for symlink in symlinks.into_iter().rev() {
        dest.write_symlink(&symlink)?;
    }

    Ok(())
}

pub fn unpack_tar_gz_carefully<T: Read + Seek, W: WriteTree>(
    body: T,
    mut dest: W,
) -> Result<()> {
    let ungz = flate2::read::MultiGzDecoder::new(body);
    let mut archive = tar::Archive::new(ungz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path: NicePathBuf = entry.path_bytes().deref().try_into()?;
        let kind = entry.header().entry_type();
        let is_executable = entry.header().mode()? & 0o100 != 0;
        use tar::EntryType::*;
        match kind {
            // In theory we could support symlinks here (and we do support them in zip
            // files, by accident because we want to support them for pybis), but lets
            // wait until someone actually needs it.
            Symlink | Link | Char | Block | Fifo => {
                bail!("sdist entry {} has unsupported type {:?}", path, kind)
            }
            Directory => dest.mkdir(&path)?,
            GNULongName | GNULongLink | GNUSparse | XGlobalHeader | XHeader => (),
            Regular | Continuous | _ => {
                dest.write_file(&path, &mut entry, is_executable)?;
            }
        }
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
            (
                "foo/bar/baz",
                "something/../../..//./stuff/../thing",
                "../../thing",
            ),
        ] {
            println!("{source} -> {target}");
            let symlink =
                NiceSymlinkPaths::new(&source.try_into().unwrap(), target.as_bytes())
                    .unwrap();
            assert_eq!(symlink.target, normalized.to_string());
        }
    }

    // XX TODO: write some tests that unpacking invalid zip files are rejected!!
}
