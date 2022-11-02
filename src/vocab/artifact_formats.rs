use super::rfc822ish::RFC822ish;
use crate::prelude::*;
use crate::tree::{unpack_zip_carefully, WriteTree};
use std::cell::RefCell;
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
pub enum ScriptType {
    GUI,
    Console,
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

struct WheelVitalSigns {
    dist_info: NicePathBuf,
    data: NicePathBuf,
    root_is_purelib: bool,
    metadata_blob: Vec<u8>,
    metadata: WheelCoreMetadata,
}

impl Wheel {
    fn get_vital_signs(&self) -> Result<WheelVitalSigns> {
        static DIST_INFO_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^([^/]*)-([^-/]*)\.dist-info/").unwrap());
        static DATA_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^([^/]*)-([^-/]*)\.data/").unwrap());

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
        let dist_str = captures.get(1).unwrap().as_str();
        let version_str = captures.get(2).unwrap().as_str();
        let data = format!("{dist_str}-{version_str}.data/");
        let dist: PackageName = dist_str.try_into()?;
        let version: Version = version_str.try_into()?;
        if (&dist, &version) != (&self.name.distribution, &self.name.version) {
            bail!("wrong name/version in directory name {dist_info}");
        }

        let mut datas = z
            .file_names()
            .filter_map(|n| DATA_NAME_RE.find(n).map(|m| m.as_str()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if datas.len() > 1 {
            bail!("found multiple .data/ directories in wheel");
        }
        if let Some(&found_data) = datas.first() {
            if found_data != data.as_str() {
                bail!("malformed .data name: expected {data}, found {found_data}");
            }
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

        Ok(WheelVitalSigns {
            dist_info: dist_info.try_into()?,
            data: data.try_into()?,
            root_is_purelib,
            metadata_blob,
            metadata,
        })
    }
}

impl BinaryArtifact for Wheel {
    type Metadata = WheelCoreMetadata;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    #[context("Reading metadata from {}", self.name)]
    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        let WheelVitalSigns { metadata_blob, metadata, .. } = self.get_vital_signs()?;
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
}

impl Pybi {
    #[context("Unpacking {}", self.name)]
    pub fn unpack<T: WriteTree>(&self, destination: T) -> Result<()> {
        // XX TODO RECORD?
        unpack_zip_carefully(&mut self.z.borrow_mut(), destination)
    }
}

impl Wheel {
    #[context("Unpacking {}", self.name)]
    pub fn unpack<W, F>(
        &self,
        paths: &HashMap<String, NicePathBuf>,
        wrap_script: F,
        mut dest: W,
    ) -> Result<()>
    where
        W: WriteTree,
        F: FnMut(Vec<u8>, ScriptType) -> Vec<u8>,
    {
        let vitals = self.get_vital_signs()?;
        let transformer = WheelTreeTransformer {
            paths: &paths,
            wrap_script: &wrap_script,
            dest: &mut dest,
            vitals,
        };
        unpack_zip_carefully(&mut self.z.borrow_mut(), transformer)?;
        Ok(())
    }
}

struct WheelTreeTransformer<'a, W, F>
where
    W: WriteTree,
    F: FnMut(Vec<u8>, ScriptType) -> Vec<u8>,
{
    paths: &'a HashMap<String, NicePathBuf>,
    wrap_script: &'a F,
    dest: &'a mut W,
    vitals: WheelVitalSigns,
}

impl<'a, W, F> WheelTreeTransformer<'a, W, F>
where
    W: WriteTree,
    F: FnMut(Vec<u8>, ScriptType) -> Vec<u8>,
{
    fn analyze_path(&self, path: &NicePathBuf) -> NicePathBuf {
        // need to check if data path is a prefix, then extract the part after that, and
        // then join with paths[whatever]
        // and for scripts
        todo!()
    }
}

impl<'a, W, F> WriteTree for WheelTreeTransformer<'a, W, F>
where
    W: WriteTree,
    F: FnMut(Vec<u8>, ScriptType) -> Vec<u8>,
{
    fn mkdir(&mut self, path: &NicePathBuf) -> Result<()> {
        todo!()
    }

    fn write_file(
        &mut self,
        path: &NicePathBuf,
        data: &mut dyn Read,
        executable: bool,
    ) -> Result<()> {
        todo!()
    }

    fn write_symlink(&mut self, symlink: &crate::tree::NiceSymlinkPaths) -> Result<()> {
        todo!()
    }
}
