use crate::{prelude::*, tree::WriteTree};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ScriptType {
    Gui,
    Console,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum FindPython {
    // from $POSY_PYTHON{,W}
    FromEnv,
    // XX TODO
    //SameDir,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ScriptPlatform {
    Unix,
    Windows,
    Both,
}

pub struct TrampolineMaker {
    strategy: FindPython,
    platform: ScriptPlatform,
}

impl TrampolineMaker {
    pub fn new(strategy: FindPython, platform: ScriptPlatform) -> TrampolineMaker {
        TrampolineMaker { strategy, platform }
    }

    pub fn make_trampoline<W: WriteTree>(
        &self,
        path: &NicePathBuf,
        script: &[u8],
        script_type: ScriptType,
        mut tree: W,
    ) -> Result<()> {
        assert_eq!(self.strategy, FindPython::FromEnv);
        if self.platform == ScriptPlatform::Unix
            || self.platform == ScriptPlatform::Both
        {
            let out = self.unix_trampoline(script, script_type);
            tree.write_file(path, &mut out.as_slice(), true)?;
        }
        if self.platform == ScriptPlatform::Windows
            || self.platform == ScriptPlatform::Both
        {
            let out = self.windows_trampoline(script, script_type);
            let mut path_str = path.to_string();
            path_str.push_str(".exe");
            let path_exe: NicePathBuf = path_str.try_into().unwrap();
            tree.write_file(&path_exe, &mut out.as_slice(), true)?;
        }
        Ok(())
    }

    fn unix_trampoline(&self, script: &[u8], script_type: ScriptType) -> Vec<u8> {
        let prefix = match script_type {
            ScriptType::Console => UNIX_TEMPLATE.into(),
            ScriptType::Gui => UNIX_TEMPLATE.replace("POSY_PYTHON", "POSY_PYTHONW"),
        };
        let mut out = prefix.into_bytes();
        out.extend_from_slice(script);
        out
    }

    fn windows_trampoline(&self, script: &[u8], script_type: ScriptType) -> Vec<u8> {
        let prefix = match script_type {
            ScriptType::Console => WINDOWS_CONSOLE,
            ScriptType::Gui => WINDOWS_GUI,
        };
        let mut suffix = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut z = zip::ZipWriter::new(&mut suffix);
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // unwrap() because we shouldn't be able to hit errors when writing to
            // memory
            z.start_file("__main__.py", options).unwrap();
            z.write_all(script).unwrap();
            z.finish().unwrap();
        }
        let mut out: Vec<u8> = prefix.into();
        out.extend(suffix.into_inner().into_iter());
        out
    }
}

const UNIX_TEMPLATE: &str = indoc::indoc! {r#"
    #!/bin/sh
    ''':'
    if [ -z "${POSY_PYTHON+x}" ]; then
        echo 'Expected $POSY_PYTHON to be set' >&2
        exit 1
    fi
    exec "${POSY_PYTHON}" "$0" "$@"
    ' '''
"#};

const WINDOWS_CONSOLE: &[u8] =
    include_bytes!("windows-trampolines/posy-trampoline-console.exe");
const WINDOWS_GUI: &[u8] =
    include_bytes!("windows-trampolines/posy-trampoline-gui.exe");
