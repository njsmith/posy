use super::rfc822ish::RFC822ish;
use crate::prelude::*;
use std::{
    cell::RefCell,
    path::{Component, Path},
};
use zip::ZipArchive;

// probably should:
// move artifact_name and core_metadata in here (at least conceptually)
// publicize &dyn ReadPlusSeek and use it everywhere instead of templatizing everything
// that touches files
// sdist, wheel, pybi each wrap a &dyn ReadPlusSeek and are constructed from that plus
// their name
// (which is necessary for sdist to figure out format)
//
// sdist and pybi will have simple extract methods, wheel extract needs to take the
// Paths map from the pybi metadata, and probably some strategy for script
// generation/#!python fixing.
//
// ...oh yeah there are also direct URL references, which might point to source trees. I
// guess that's a 4th artifact type?

pub struct Sdist {
    // TODO
}
pub struct Wheel {
    name: WheelName,
    z: RefCell<ZipArchive<Box<dyn ReadPlusSeek>>>,
}
pub struct Pybi {
    name: PybiName,
    z: RefCell<ZipArchive<Box<dyn ReadPlusSeek>>>,
}

pub trait Artifact: Sized {
    type Name: Clone + UnwrapFromArtifactName;

    fn new(name: Self::Name, f: Box<dyn ReadPlusSeek>) -> Result<Self>;
}

impl Artifact for Wheel {
    type Name = WheelName;

    fn new(name: Self::Name, f: Box<dyn ReadPlusSeek>) -> Result<Self> {
        Ok(Wheel {
            name,
            z: RefCell::new(ZipArchive::new(f)?),
        })
    }
}

impl Artifact for Pybi {
    type Name = PybiName;

    fn new(name: Self::Name, f: Box<dyn ReadPlusSeek>) -> Result<Self> {
        Ok(Pybi {
            name,
            z: RefCell::new(ZipArchive::new(f)?),
        })
    }
}

pub trait BinaryArtifact: Artifact {
    type Metadata;

    // used to parse standalone METADATA files, like we might have cached or get from
    // PEP 658 (once it's implemented). Eventually we might want to split this off into
    // its own trait, because we'll want to implement it for sdists once PEP 643 is
    // useful (https://github.com/pypa/setuptools/issues/2685)
    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata>;

    // used to parse from an actual binary artifact; finds the right metadata directory,
    // and also parses/validates the WHEEL/PYBI metadata, not just the core metadata.
    // Also returns the raw core metadata file contents, because sometimes we want to
    // use this to pull out the metadata from a remote artifact without downloading the
    // whole thing, and also cache the core metadata locally for next time.
    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)>;

    fn unpack(&self, destination: &Path) -> Result<()>;
}

fn parse_format_metadata_and_check_version(
    input: &[u8],
    version_field: &str,
) -> Result<RFC822ish> {
    let input: &str = std::str::from_utf8(input)?;
    let mut parsed = RFC822ish::parse(&input)?;

    let version = parsed.take_the(version_field)?;
    if !version.starts_with("1.") {
        bail!("unsupported {}: {:?}", version_field, version);
    }

    Ok(parsed)
}

#[context("extracting {name}")]
fn slurp_from_zip<'a, T: Read + Seek>(
    z: &'a mut ZipArchive<T>,
    name: &str,
) -> Result<Vec<u8>> {
    Ok(slurp(&mut z.by_name(name)?)?)
}

// XX TODO: rewrite this
// - I'm not at all convinced that the symlink target sanitization is correct, e.g. what
//   if 'source' ends in / (or what if it doesn't? foo/bar.join(baz) is what? do I need
//   to call .parent? what if .parent is None?) need to pull it out and make it testable
// - should do our own extraction b/c wheel wants to redirect stuff
// - .enclosed_name allows stuff like foo/../bar, and probably we just want to error out
//   if anyone has a filename like that
fn unpack<T: Read + Seek>(z: &mut ZipArchive<T>, dest: &Path) -> Result<()> {
    // As of zipfile v0.15.3, this is symlink-oblivious: it'll just unpack symlinks as
    // regular files with the link target as their contents.
    z.extract(&dest)?;
    // So we come in after to fix up the symlinks. There are a few nasty issues with
    // symlinks that we have to be careful about:
    //
    // - foo -> baz, foo/bar regular entry or symlink: does 'bar' get put in 'baz', or
    //   what?
    //
    //   Prevented by: the initial symlink-oblivious extract step will fail here, b/c
    //   'foo' appears as both a "regular file" and a directory.
    //
    // - symlinks pointing outside the extracted archive: weird, suspicious, don't like
    //   it.
    //
    //   Prevented by: careful examination of the symlink destination path.
    #[cfg(unix)]
    for i in 0..z.len() {
        let mut zip_file = z.by_index(i)?;
        let is_symlink = match zip_file.unix_mode() {
            Some(mode) => mode & 0xf000 == 0xa000,
            None => false,
        };
        println!("{}: is_symlink = {}", zip_file.name(), is_symlink);
        if is_symlink {
            let source = zip_file
                .enclosed_name()
                .ok_or(anyhow!("bad symlink source: {}", zip_file.name()))?
                .to_path_buf();
            let raw_target = slurp(&mut zip_file)?;
            let target = std::str::from_utf8(&raw_target)?;
            if target.contains('\0') {
                bail!("bad symlink: {} -> {:?}", zip_file.name(), target);
            }
            let target = Path::new(target);
            let resolved = source.join(&target);
            // count depth of resolved path: add 1 for each directory segment, subtract
            // one for each '..', it should never go negative.
            let mut depth = 0u32;
            for component in resolved.components() {
                match component {
                    Component::Prefix(_) | Component::RootDir => {
                        bail!("invalid symlink target {}", target.display())
                    }
                    Component::CurDir => (),
                    Component::ParentDir => {
                        depth = depth.checked_add(1).ok_or(anyhow!(
                            "invalid symlink target {}",
                            target.display()
                        ))?;
                    }
                    Component::Normal(_) => {
                        depth = depth.checked_sub(1).ok_or(anyhow!(
                            "invalid symlink target {}",
                            target.display()
                        ))?;
                    },
                }
            }
            if depth <= 0 {
                bail!(
                    "symlink target {} points outside archive directory",
                    target.display()
                );
            }
            let full_source = dest.join(&source);
            println!(
                "symlinking {} -> {}",
                full_source.display(),
                target.display()
            );
            std::fs::remove_file(&full_source)?;
            std::os::unix::fs::symlink(&target, &full_source)?;
        }
    }
    Ok(())
}

impl BinaryArtifact for Wheel {
    type Metadata = WheelCoreMetadata;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    #[context("Reading metadata from {}", self.name)]
    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        static DIST_INFO_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^([^/]*)-([^-/]*)\.dist-info/").unwrap());

        let mut z = self.z.borrow_mut();

        let dist_info;
        {
            let mut dist_infos = z
                .file_names()
                .filter_map(|n| DIST_INFO_NAME_RE.find(n).map(|m| m.as_str()))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();

            dist_info = match dist_infos.pop() {
                Some(d) => d.to_owned(),
                None => bail!("no .dist-info/ directory found in wheel"),
            };
            if !dist_infos.is_empty() {
                bail!("found multiple .dist-info/ directories in wheel");
            }
        }
        let captures = DIST_INFO_NAME_RE
            .captures(dist_info.as_str())
            .ok_or(anyhow!("malformed .dist-info name {dist_info}"))?;
        let dist: PackageName = captures.get(1).unwrap().as_str().try_into()?;
        let version: Version = captures.get(2).unwrap().as_str().try_into()?;
        if (&dist, &version) != (&self.name.distribution, &self.name.version) {
            bail!("wrong name/version in directory name {dist_info}");
        }

        let wheel_path = format!("{dist_info}WHEEL");
        let wheel_metadata = slurp_from_zip(&mut z, &wheel_path)?;

        let mut parsed =
            parse_format_metadata_and_check_version(&wheel_metadata, "Wheel-Version")?;

        let root_is_purelib = match &parsed.take_the("Root-Is-Purelib")?[..] {
            "true" => true,
            "false" => false,
            other => bail!(
                "Expected 'true' or 'false' for Root-Is-Purelib, not {}",
                other,
            ),
        };
        // XX refactor so this is accessible to the unpacking code once it's written
        // (of course it will also need entry points and RECORD and stuff)
        drop(root_is_purelib);

        let metadata_path = format!("{dist_info}METADATA");
        let metadata_blob = slurp_from_zip(&mut z, &metadata_path)?;

        let metadata: WheelCoreMetadata = metadata_blob.as_slice().try_into()?;

        if metadata.name != self.name.distribution {
            bail!(
                "name mismatch between {dist_info}METADATA and filename ({} != {}",
                metadata.name.as_given(),
                self.name.distribution.as_given()
            );
        }
        if metadata.version != self.name.version {
            bail!(
                "version mismatch between {dist_info}METADATA and filename ({} != {})",
                metadata.version,
                self.name.version
            );
        }

        Ok((metadata_blob, metadata))
    }

    #[context("Unpacking {}", self.name)]
    fn unpack(&self, destination: &Path) -> Result<()> {
        // XX TODO RECORD? spreading?
        unpack(&mut self.z.borrow_mut(), destination)
    }
}

impl BinaryArtifact for Pybi {
    type Metadata = PybiCoreMetadata;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        let mut z = self.z.borrow_mut();
        let format_metadata_blob = slurp_from_zip(&mut z, "pybi-info/PYBI")?;
        parse_format_metadata_and_check_version(&format_metadata_blob, "Pybi-Version")?;
        let metadata_blob = slurp_from_zip(&mut z, "pybi-info/METADATA")?;
        let metadata: PybiCoreMetadata = metadata_blob.as_slice().try_into()?;
        if metadata.name != self.name.distribution {
            bail!(
                "name mismatch between pybi/METADATA and filename ({} != {})",
                metadata.name.as_given(),
                self.name.distribution.as_given()
            );
        }
        if metadata.version != self.name.version {
            bail!(
                "version mismatch between pybi/METADATA and filename ({} != {})",
                metadata.version,
                self.name.version
            );
        }
        Ok((metadata_blob, metadata))
    }

    #[context("Unpacking {}", self.name)]
    fn unpack(&self, destination: &Path) -> Result<()> {
        // XX TODO RECORD?
        unpack(&mut self.z.borrow_mut(), destination)
    }
}
