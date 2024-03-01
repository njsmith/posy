use super::rfc822ish::RFC822ish;
use crate::package_db::ArtifactInfo;
use crate::prelude::*;
use crate::trampolines::{ScriptType, TrampolineMaker};
use crate::tree::{unpack_tar_gz_carefully, unpack_zip_carefully, WriteTree};
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
// guess that's a 4th artifact type? Or a variant on Sdist? not sure.

pub struct Sdist {
    name: SdistName,
    body: RefCell<Box<dyn ReadPlusSeek>>,
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
    fn name(&self) -> &Self::Name;
}

impl Artifact for Sdist {
    type Name = SdistName;

    fn new(name: Self::Name, body: Box<dyn ReadPlusSeek>) -> Result<Self> {
        Ok(Sdist {
            name,
            body: body.into(),
        })
    }

    fn name(&self) -> &Self::Name {
        &self.name
    }
}

impl Sdist {
    pub fn unpack<T: WriteTree>(&self, destination: &mut T) -> Result<()> {
        context!("Unpacking {}", self.name);
        let mut boxed = self.body.borrow_mut();
        let body = boxed.as_mut();
        match self.name.format {
            SdistFormat::Zip => {
                unpack_zip_carefully(&mut ZipArchive::new(body)?, destination)
            }
            SdistFormat::TarGz => unpack_tar_gz_carefully(body, destination),
        }
    }
}

impl Artifact for Wheel {
    type Name = WheelName;

    fn new(name: Self::Name, f: Box<dyn ReadPlusSeek>) -> Result<Self> {
        Ok(Wheel {
            name,
            z: RefCell::new(ZipArchive::new(f)?),
        })
    }

    #[inline]
    fn name(&self) -> &Self::Name {
        &self.name
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

    fn name(&self) -> &Self::Name {
        &self.name
    }
}

// This should add a 'Name: BinaryName' bound on Artifact::Name, but that's not stable
// yet: https://github.com/rust-lang/rust/issues/52662
pub trait BinaryArtifact: Artifact {
    type Metadata;
    type Platform: Platform;
    type Builder<'a>;

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

    // These are only meaningful for Wheel, because only Wheel has an sdist format. But
    // we want to call these from PackageDB methods that are generic over arbitrary
    // BinaryArtifacts, and don't know even know how to recognize an Sdist ArtifactInfo
    // from our index. So our trick is that the generic methods just call these on every
    // ArtifactInfo, and the Wheel version recognizes Sdists and builds them, and for
    // every other case they return Ok(None).
    //
    // (Also, if we ever get an sdist format for pybis, then we're prepared!)

    fn locally_built_metadata(
        ctx: &Self::Builder<'_>,
        ai: &ArtifactInfo,
    ) -> Option<Result<(Vec<u8>, Self::Metadata)>>;

    fn locally_built_binary(
        ctx: &Self::Builder<'_>,
        ai: &ArtifactInfo,
        platform: &Self::Platform,
    ) -> Option<Result<Self>>;
}

fn parse_format_metadata_and_check_version(
    input: &[u8],
    version_field: &str,
) -> Result<RFC822ish> {
    let input: &str = std::str::from_utf8(input)?;
    let mut parsed = RFC822ish::parse(input)?;

    let version = parsed.take_the(version_field)?;
    if !version.starts_with("1.") {
        bail!("unsupported {}: {:?}", version_field, version);
    }

    Ok(parsed)
}

fn slurp_from_zip<T: Read + Seek>(
    z: &mut ZipArchive<T>,
    name: &str,
) -> Result<Vec<u8>> {
    context!("extracting {name}");
    slurp(&mut z.by_name(name)?)
}

struct WheelVitals {
    dist_info: String,
    data: String,
    root_is_purelib: bool,
    metadata_blob: Vec<u8>,
    metadata: WheelCoreMetadata,
}

impl Wheel {
    /// Little helper for picking out the .dist-info or .data directory from an
    /// iterator.
    pub fn find_special_wheel_dir<'a, I, S>(
        names: I,
        name: &PackageName,
        version: &Version,
        suffix: &str,
    ) -> Result<Option<S>>
    where
        I: IntoIterator<Item = S>,
        S: 'a + AsRef<str>,
    {
        static SPECIAL_WHEEL_DIR_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^(.*)-(.*)\..*").unwrap());

        assert!(suffix.starts_with('.'));

        let mut candidates = names
            .into_iter()
            .filter(|n| n.as_ref().ends_with(suffix))
            .collect::<Vec<_>>();

        let candidate = match candidates.pop() {
            Some(c) => c,
            None => return Ok(None),
        };
        if !candidates.is_empty() {
            bail!("found multiple {suffix}/ directories in wheel");
        }
        let candidate_str = candidate.as_ref();
        context!("parsing wheel directory {candidate_str}");
        match SPECIAL_WHEEL_DIR_RE.captures(candidate_str) {
            None => bail!("invalid {suffix} name: couldn't find name/version"),
            Some(captures) => {
                let got_name: PackageName =
                    captures.get(1).unwrap().as_str().try_into()?;
                if name != &got_name {
                    bail!(
                        "wrong name in {candidate_str}: expected {}",
                        name.as_given()
                    );
                }
                let got_version: Version =
                    captures.get(2).unwrap().as_str().try_into()?;
                if version != &got_version {
                    bail!("wrong version in {candidate_str}: expected {version}");
                }
                Ok(Some(candidate))
            }
        }
    }

    fn get_vitals(&self) -> Result<WheelVitals> {
        let mut z = self.z.borrow_mut();

        let dist_info;
        let data;
        {
            let top_levels = z
                .file_names()
                .map(|n| {
                    if let Some((base, _rest)) = n.split_once(['/', '\\']) {
                        base
                    } else {
                        n
                    }
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();

            dist_info = Wheel::find_special_wheel_dir(
                &top_levels,
                &self.name.distribution,
                &self.name.version,
                ".dist-info",
            )?
            .ok_or(eyre!(".dist-info/ missing"))?
            .to_string();

            if let Some(d) = Wheel::find_special_wheel_dir(
                &top_levels,
                &self.name.distribution,
                &self.name.version,
                ".data",
            )? {
                data = d.to_string();
            } else {
                // synthesize a fake .data directory, to reduce special cases later.
                // (This way we just don't have any files in the directory; and
                // synthetic files like command entrypoints have somewhere to be.)
                data =
                    format!("{}.data", dist_info.strip_suffix(".dist-info").unwrap());
            }
        }

        let wheel_path = format!("{dist_info}/WHEEL");
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

        let metadata_path = format!("{dist_info}/METADATA");
        let metadata_blob = slurp_from_zip(&mut z, &metadata_path)?;

        let metadata: WheelCoreMetadata = metadata_blob.as_slice().try_into()?;

        if metadata.name != self.name.distribution {
            bail!(
                "name mismatch between {dist_info}/METADATA and filename ({} != {}",
                metadata.name.as_given(),
                self.name.distribution.as_given()
            );
        }
        if metadata.version != self.name.version {
            bail!(
                "version mismatch between {dist_info}/METADATA and filename ({} != {})",
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

    fn metadata(&self) -> Result<(Vec<u8>, Self::Metadata)> {
        context!("Reading metadata from {}", self.name);
        let WheelVitals {
            metadata_blob,
            metadata,
            ..
        } = self.get_vitals()?;
        Ok((metadata_blob, metadata))
    }

    type Builder<'a> = crate::package_db::WheelBuilder<'a>;

    fn locally_built_metadata(
        builder: &Self::Builder<'_>,
        ai: &ArtifactInfo,
    ) -> Option<Result<(Vec<u8>, Self::Metadata)>> {
        if ai.is::<Sdist>() {
            Some(builder.locally_built_metadata(ai))
        } else {
            None
        }
    }

    fn locally_built_binary(
        builder: &Self::Builder<'_>,
        ai: &ArtifactInfo,
        platform: &Self::Platform,
    ) -> Option<Result<Self>> {
        if ai.is::<Sdist>() {
            Some(builder.locally_built_wheel(ai, platform))
        } else {
            None
        }
    }
}

impl BinaryArtifact for Pybi {
    type Metadata = PybiCoreMetadata;
    type Platform = PybiPlatform;
    // Pybis can't be built from source (at least for now)
    type Builder<'a> = ();

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

    fn locally_built_metadata(
        _ctx: &Self::Builder<'_>,
        _ai: &ArtifactInfo,
    ) -> Option<Result<(Vec<u8>, Self::Metadata)>> {
        None
    }

    fn locally_built_binary(
        _ctx: &Self::Builder<'_>,
        _ai: &ArtifactInfo,
        _platform: &Self::Platform,
    ) -> Option<Result<Self>> {
        None
    }
}

impl Pybi {
    pub fn unpack<T: WriteTree>(&self, destination: &mut T) -> Result<()> {
        context!("Unpacking {}", self.name);
        // XX TODO RECORD?
        unpack_zip_carefully(&mut self.z.borrow_mut(), destination)
    }
}

fn script_for_entrypoint(entry: &Entrypoint, script_type: ScriptType) -> Vec<u8> {
    let w = if script_type == ScriptType::Gui {
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
    pub fn unpack<W: WriteTree>(
        &self,
        paths: &HashMap<String, NicePathBuf>,
        trampoline_maker: &TrampolineMaker,
        mut dest: W,
    ) -> Result<()> {
        context!("Unpacking {}", self.name);
        let vitals = self.get_vitals()?;
        let mut transformer = WheelTreeTransformer {
            paths,
            trampoline_maker,
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
                        let name =
                            format!("{}/scripts/{}", vitals.data, entrypoint.name);
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
            write_scripts("gui_scripts", ScriptType::Gui)?;
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
            .ok_or_else(|| eyre!("unrecognized wheel file category {category}"))?;
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
        if let Some((fixed_path, _)) = self.analyze_path(path)? {
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
        if let Some((fixed_path, is_script)) = self.analyze_path(path)? {
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
                        ScriptType::Gui
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
