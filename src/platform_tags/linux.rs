use crate::prelude::*;

use std::fs::File;
use std::io::Write;
use std::os::unix::{fs::PermissionsExt, io::AsRawFd};
use std::path::PathBuf;
use std::process::Command;

// Ordered from most-preferred to least-preferred (so e.g. 64-bit platforms should
// usually go first)
static GLIBC_DETECTORS: Lazy<Vec<(&str, &[u8])>> = Lazy::new(|| {
    let mut glibc_detectors: Vec<(&str, &[u8])> = Vec::new();

    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    {
        glibc_detectors.push((
            "x86_64",
            include_bytes!("linux-glibc-detectors/glibc-detector-x86_64"),
        ));
        glibc_detectors.push((
            "i686",
            include_bytes!("linux-glibc-detectors/glibc-detector-i686"),
        ));
    }

    #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
    {
        glibc_detectors.push((
            "aarch64",
            include_bytes!("linux-glibc-detectors/glibc-detector-aarch64"),
        ));
        glibc_detectors.push((
            "armv7l",
            include_bytes!("linux-glibc-detectors/glibc-detector-armv7l"),
        ));
    }

    #[cfg(any(target_arch = "powerpc64"))]
    {
        glibc_detectors.push((
            "ppc64le",
            include_bytes!("linux-glibc-detectors/glibc-detector-ppc64le"),
        ));
    }

    #[cfg(any(target_arch = "s390x"))]
    {
        glibc_detectors.push((
            "s390x",
            include_bytes!("linux-glibc-detectors/glibc-detector-s390x"),
        ));
    }

    glibc_detectors
});

static GLIBC_VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([0-9]+)\.([0-9]+)").unwrap());

fn glibc_tags(py_arch: &str, detector: &[u8]) -> Result<Vec<String>> {
    // This is a stupid hack to run 'detector' as an executable, with the guarantees
    // that (1) we can't accidentally leak it (the OS will clean it up for us if we
    // crash unexpectedly), (2) we completely avoid all the nasty race conditions /
    // potential security holes / etc. that happen if you try to write a temp file and
    // then re-open it by name. The downsides are it requires proc() (could possibly be
    // avoided via memfd_create+F_SEAL_WRITE+fexecve?), and it might make some security
    // scanner freak out at some point because worms like to use this kind of trick too.
    // But on the other hand, it was fun to write, and it's not like I'm getting paid
    // for this.
    let mut f = tempfile::tempfile()?;
    f.write_all(detector)?;
    let permissions = PermissionsExt::from_mode(0o700);
    f.set_permissions(permissions)?;
    // Have to re-open because exec() requires that the file has no open writers
    let f_readonly = File::open(format!("/proc/self/fd/{}", f.as_raw_fd()))?;
    drop(f);
    let output =
        Command::new(format!("/proc/self/fd/{}", f_readonly.as_raw_fd())).output()?;
    if !output.status.success() {
        // XX log something, but this is not an error
        println!("non-zero return for {}: {}", py_arch, output.status);
        Ok(vec![])
    } else {
        let output_text = String::from_utf8_lossy(&output.stdout);
        match GLIBC_VERSION_RE.captures(&output_text) {
            None => {
                bail!("unexpected glibc version number: {:?}", output.stdout)
            }
            Some(captures) => {
                let major: u32 = captures.get(1).unwrap().as_str().parse()?;
                let minor: u32 = captures.get(2).unwrap().as_str().parse()?;
                if major > 2 {
                    bail!(
                        "glibc 3? I don't understand glibc 3 (got version: {})",
                        output_text.trim()
                    )
                };
                Ok((5..=minor)
                    .rev()
                    .map(|n| format!("manylinux_{}_{}_{}", major, n, py_arch))
                    .collect())
            }
        }
    }
}

// maps musl platform names to python arch tags
// also ordered from most-preferred to least-preferred
static MUSL_ARCH_MAP: &[(&str, &str)] = &[
    ("x86_64", "x86_64"),
    ("aarch64", "aarch64"),
    ("i386", "i686"),
    ("armhf", "armv7l"),
    ("powerpc64le", "ppc64le"),
    ("s390x", "s390x"),
];

static MUSL_VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Version ([0-9]+)\.([0-9]+)").unwrap());

fn musl_tags(loader: &PathBuf, py_arch: &str) -> Result<Vec<String>> {
    match Command::new(&loader).output() {
        Err(e) => bail!("failed to execute: {}", e),
        Ok(output) => {
            // don't check output.status, because it's expected to return
            // non-zero
            let output_text = String::from_utf8_lossy(&output.stderr);
            match MUSL_VERSION_RE.captures(&output_text) {
                None => bail!("couldn't find version string",),
                Some(captures) => {
                    let major: u32 = captures.get(1).unwrap().as_str().parse()?;
                    let minor: u32 = captures.get(2).unwrap().as_str().parse()?;
                    Ok((0..=minor)
                        .rev()
                        .map(|n| format!("musllinux_{}_{}_{}", major, n, py_arch))
                        .collect())
                }
            }
        }
    }
}

pub fn platform_tags() -> Result<Vec<String>> {
    let mut all_tags: Vec<String> = Vec::new();

    for (py_arch, detector) in GLIBC_DETECTORS.iter() {
        match glibc_tags(py_arch, detector) {
            Ok(mut tags) => all_tags.append(&mut tags),
            // XX use logging instead
            Err(e) => println!("error checking glibc version on {}: {}", py_arch, e),
        }
    }

    for (musl_arch, py_arch) in MUSL_ARCH_MAP {
        let loader: PathBuf = format!("/lib/ld-musl-{}.so.1", musl_arch).into();
        if loader.exists() {
            match musl_tags(&loader, py_arch) {
                Ok(mut tags) => all_tags.append(&mut tags),
                // XX use logging instead
                Err(e) => println!(
                    "error fetching musl metadata from {}: {}",
                    loader.to_string_lossy(),
                    e
                ),
            }
        }
    }

    Ok(all_tags)
}
