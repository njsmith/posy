use super::rfc822ish::RFC822ish;
use crate::prelude::*;
use crate::trampolines::{TrampolineMaker, ScriptType};
use crate::tree::{unpack_zip_carefully, WriteTree};
use std::cell::RefCell;
use std::io::{BufRead, BufReader};
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
    pub name: WheelName,
    z: RefCell<ZipArchive<Box<dyn ReadPlusSeek>>>,
}
pub struct Pybi {
    pub name: PybiName,
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

// This should add a 'Name: BinaryName' bound on Artifact::Name, but that's not stable
// yet: https://github.com/rust-lang/rust/issues/52662
pub trait BinaryArtifact: Artifact {
    type Metadata;
    type Platform: Platform;

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

struct WheelVitals {
    dist_info: String,
    data: String,
    root_is_purelib: bool,
    metadata_blob: Vec<u8>,
    metadata: WheelCoreMetadata,
}

impl Wheel {
    fn get_vitals(&self) -> Result<WheelVitals> {
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

        let datas = z
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

        Ok(WheelVitals {
            // go through NicePathBuf to make sure these are nice and normalized
            dist_info: TryInto::<NicePathBuf>::try_into(dist_info)?.to_string(),
            data: TryInto::<NicePathBuf>::try_into(data)?.to_string(),
            root_is_purelib,
            metadata_blob,
            metadata,
        })
    }
}

impl BinaryArtifact for Wheel {
    type Metadata = WheelCoreMetadata;
    type Platform = WheelPlatform;

    fn parse_metadata(value: &[u8]) -> Result<Self::Metadata> {
        value.try_into()
    }

    #[context("Reading metadata from {}", self.name)]
    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        let WheelVitals {
            metadata_blob,
            metadata,
            ..
        } = self.get_vitals()?;
        Ok((metadata_blob, metadata))
    }
}

impl BinaryArtifact for Pybi {
    type Metadata = PybiCoreMetadata;
    type Platform = PybiPlatform;

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
    pub fn unpack<T: WriteTree>(&self, destination: &mut T) -> Result<()> {
        // XX TODO RECORD?
        unpack_zip_carefully(&mut self.z.borrow_mut(), destination)
    }
}

fn script_for_entrypoint(entry: &Entrypoint, script_type: ScriptType) -> Vec<u8> {
    let w = if script_type == ScriptType::GUI {
        "w"
    } else {
        ""
    };
    let Entrypoint { module, object, .. } = entry;
    let suffix = if let Some(object) = object {
        format!(".{object}")
    } else {
        "".into()
    };
    indoc::formatdoc! {r###"
         #!python{w}
         # -*- coding: utf-8 -*-
         import sys
         import {module}
         if __name__ == "__main__":
             if sys.argv[0].endswith(".exe"):
                 sys.argv[0] = sys.argv[0][:-4]
             sys.exit({module}{suffix}())
    "###}
    .into()
}

impl Wheel {
    // XX TODO RECORD?
    #[context("Unpacking {}", self.name)]
    pub fn unpack<W: WriteTree>(
        &self,
        paths: &HashMap<String, NicePathBuf>,
        trampoline_maker: &TrampolineMaker,
        mut dest: W,
    ) -> Result<()>
    {
        let vitals = self.get_vitals()?;
        let mut transformer = WheelTreeTransformer {
            paths: &paths,
            trampoline_maker: &trampoline_maker,
            dest: &mut dest,
            vitals: &vitals,
        };
        let mut z = self.z.borrow_mut();
        unpack_zip_carefully(&mut z, &mut transformer)?;
        let mut installer: &[u8] = b"posy\n";
        transformer.write_file(
            &format!("{}/INSTALLER", vitals.dist_info)
                .as_str()
                .try_into()
                .unwrap(),
            &mut installer,
            false,
        )?;

        if let Ok(entry_points) = slurp_from_zip(
            &mut z,
            format!("{}/{}", vitals.dist_info, "entry_points.txt").as_str(),
        ) {
            let entry_points = parse_entry_points(std::str::from_utf8(&entry_points)?)?;

            let mut write_scripts = |name, script_type| -> Result<()> {
                if let Some(script_entrypoints) = entry_points.get(name) {
                    for entrypoint in script_entrypoints {
                        let body = script_for_entrypoint(entrypoint, script_type);
                        let name = format!("{}/scripts/{}", vitals.data, entrypoint.name);
                        transformer.write_file(
                            &name.try_into()?,
                            &mut &body[..],
                            true,
                        )?;
                    }
                }
                Ok(())
            };

            write_scripts("console_scripts", ScriptType::Console)?;
            write_scripts("gui_scripts", ScriptType::GUI)?;
        }
        Ok(())
    }
}

struct WheelTreeTransformer<'a, W: WriteTree> {
    paths: &'a HashMap<String, NicePathBuf>,
    trampoline_maker: &'a TrampolineMaker,
    dest: &'a mut W,
    vitals: &'a WheelVitals,
}

impl<'a, W> WheelTreeTransformer<'a, W>
where
    W: WriteTree,
{
    fn analyze_path(&self, path: &NicePathBuf) -> Result<Option<(NicePathBuf, bool)>> {
        // need to check if data path is a prefix, then extract the part after that, and
        // then join with paths[whatever]
        // and for scripts
        let (category, range) = if path.pieces().get(0) == Some(&self.vitals.data) {
            if let Some(category) = path.pieces().get(1) {
                (category.as_str(), 2..)
            } else {
                // the .data directory itself; discard
                return Ok(None);
            }
        } else {
            (
                if self.vitals.root_is_purelib {
                    "purelib"
                } else {
                    "platlib"
                },
                0..,
            )
        };
        let basepath = self
            .paths
            .get(category)
            .ok_or_else(|| anyhow!("unrecognized wheel file category {category}"))?;
        Ok(Some((
            basepath.join(&path.slice(range)),
            category == "scripts",
        )))
    }
}

impl<'a, W> WriteTree for WheelTreeTransformer<'a, W>
where
    W: WriteTree,
{
    fn mkdir(&mut self, path: &NicePathBuf) -> Result<()> {
        if let Some((fixed_path, _)) = self.analyze_path(&path)? {
            self.dest.mkdir(&fixed_path)
        } else {
            Ok(())
        }
    }

    fn write_file(
        &mut self,
        path: &NicePathBuf,
        mut data: &mut dyn Read,
        _executable: bool,
    ) -> Result<()> {
        if let Some((fixed_path, is_script)) = self.analyze_path(&path)? {
            if is_script {
                // use BufReader to "peek" into the start of the executable. Some wheels
                // contain large compiled binaries as "scripts", so it's nicer not to
                // load the whole thing into memory until we know what we're dealing
                // with.
                let mut bufread = BufReader::new(&mut data);
                let script_start = bufread.fill_buf()?;
                if script_start.starts_with(b"#!python") {
                    // it's some kind of script, but which kind?
                    let script_type = if script_start.starts_with(b"#!pythonw") {
                        ScriptType::GUI
                    } else {
                        ScriptType::Console
                    };
                    // discard #! line
                    bufread.read_line(&mut String::new())?;
                    let script = slurp(&mut bufread)?;
                    self.trampoline_maker.make_trampoline(
                        &fixed_path,
                        &script,
                        script_type,
                        &mut self.dest,
                    )?;
                } else {
                    self.dest.write_file(&fixed_path, &mut bufread, true)?;
                }
            } else {
                self.dest.write_file(&fixed_path, data, false)?;
            }
        }
        Ok(())
    }

    fn write_symlink(
        &mut self,
        _symlink: &crate::tree::NiceSymlinkPaths,
    ) -> Result<()> {
        bail!("symlinks not supported in wheels");
    }
}
