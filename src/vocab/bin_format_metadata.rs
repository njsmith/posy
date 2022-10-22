use crate::prelude::*;
use zip::ZipArchive;
use std::cell::RefCell;
use super::rfc822ish::RFC822ish;

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
    type Name;

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

fn find_dist_info<T: Read + Seek>(z: &mut ZipArchive<T>) -> Result<&str> {
    let dist_infos: Vec<&str> = z.file_names().filter(|n| n.ends_with(".dist-info")).collect();
    let dist_info = match dist_infos.pop() {
        Some(info_dir) => info_dir,
        None => bail!("no .dist-info/ directory found in wheel"),
    };
    if !dist_infos.is_empty() {
        bail!("found multiple .dist-info/ directories in wheel");
    }
    Ok(dist_info)
}

impl BinaryArtifact for Wheel {
    type Metadata = WheelCoreMetadata;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        let mut z = self.z.borrow_mut();

        let dist_infos: Vec<&str> = z.file_names().filter(|n| n.ends_with(".dist-info")).collect();
        let dist_info = match dist_infos.pop() {
            Some(d) => d,
            None => bail!("no .dist-info/ directory found in wheel"),
        };
        if !dist_infos.is_empty() {
            bail!("found multiple .dist-info/ directories in wheel");
        }
        let parsed_dist_info: DistInfoDirName = dist_info.try_into()?;
        if parsed_dist_info.distribution != self.name.distribution {
            bail!("name mismatch between {dist_info} and {self.name}");
        }
        if parsed_dist_info.version != self.name.version {
            bail!("version mismatch between {dist_info} and {self.name}");
        }


        let wheel_path = format!("{dist_info}/WHEEL");
        let wheel_metadata = slurp(&mut z.by_name(&wheel_path)?)?;

        let mut parsed = parse_format_metadata_and_check_version(
            &wheel_metadata, "Wheel-Version"
        )?;

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

        let metadata_path = format!("{dist_info}/METADATA");
        let metadata_blob = slurp(&mut z.by_name(&metadata_path)?)?;

        let metadata: WheelCoreMetadata = metadata_blob.as_slice().try_into()?;

        if metadata.name != self.name.distribution {
            bail!("name mismatch between {dist_info}/METADATA and filename ({metadata.name} != {self.name.distribution}");
        }
        if metadata.version != self.name.version {
            bail!("version mismatch between {dist_info}/METADATA and filename ({metadata.version} != {self.name.version}");
        }

        Ok((metadata_blob, metadata))
    }
}

impl BinaryArtifact for Pybi {
    type Metadata = PybiCoreMetadata;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        let mut z = self.z.borrow_mut();
        let format_metadata_blob = slurp(&mut z.by_name("pybi/PYBI")?)?;
        parse_format_metadata_and_check_version(&format_metadata_blob, "Pybi-Version")?;
        let metadata_blob = slurp(&mut z.by_name("pybi/METADATA")?)?;
        let metadata: PybiCoreMetadata = metadata_blob.as_slice().try_into()?;
        if metadata.name != self.name.distribution {
            bail!("name mismatch between pybi/METADATA and filename ({metadata.name} != {self.name.distribution}");
        }
        if metadata.version != self.name.version {
            bail!("version mismatch between pybi/METADATA and filename ({metadata.version} != {self.name.version}");
        }
        Ok((metadata_blob, metadata))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_wheel_metadata() {
        let wm: WheelCoreMetadata = b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\n"
            .try_into()
            .unwrap();
        assert!(wm.root_is_purelib);

        let wm = WheelMetadata::parse(b"Wheel-Version: 1.0\nRoot-Is-Purelib: false\n")
            .unwrap();
        assert!(!wm.root_is_purelib);

        assert!(WheelMetadata::parse(b"").is_err());
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 2.0\nRoot-Is-Purelib: true\n")
                .is_err()
        );
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 1.0\nRoot-Is-Purelib: maybe\n")
                .is_err()
        );
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 1.9\nRoot-Is-Purelib: true\n")
                .is_ok()
        );
    }

    #[test]
    fn test_pybi_metadata() {
        assert!(PybiMetadata::parse(b"").is_err());
        assert!(PybiMetadata::parse(b"Pybi-Version: 1.0\n").is_ok());
        assert!(PybiMetadata::parse(b"Pybi-Version: 1.9\n").is_ok());
        assert!(PybiMetadata::parse(b"Pybi-Version: 2.0\n").is_err());
    }
}
