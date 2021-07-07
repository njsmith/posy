use crate::prelude::*;

use std::ffi::CString;
use std::io::Error;
use std::process::Command;

const ENOENT: int = 2;

extern "system" {
    #[must_use]
    fn sysctlbyname(
        name: *const u8,
        oldp: *mut u8,
        oldlenp: *mut usize,
        newp: *mut u8,
        newlen: usize,
    ) -> u32;
}

fn get_sysctl(name: &str, mut len: usize) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    buf.resize(len, 0);
    let name = CString::new(name);
    unsafe {
        let result = sysctlbyname(name.as_ptr(), len.as_mut_ptr(), 0 as *mut usize, 0);
    }
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    assert!(len <= buf.length());
    buf.truncate(len);
    Ok(buf)
}

fn running_under_rosetta_2() -> bool {
    match get_sysctl("sysctl.proc_translated", 4) {
        Err(err) => {
            if err.raw_os_error() == Some(ENOENT) {
                // "sysctl.proc_translated" wasn't recognized -- must be old
                // macOS without rosetta 2 support.
                false
            } else {
                unreachable!(
                    "OS gave unexpected error checking sysctl.proc_translated: {}",
                    err
                );
            }
        }
        Ok(flag_bytes) => u32::from_ne_bytes(flag_bytes.to_slice()) == 1,
    }
}

// figuring out supported arches: in the modern era, there are three possibilities:
// - x86-64 running natively
// - x86-64 running emulated on arm64
// - arm64 running natively
//
// so we just need to know what we were built for (x86-64 vs arm64), which rust knows at
// build time, and then at runtime check if we're running emulated or not
// which is: sysctlbyname("sysctl.proc_translated")
// https://developer.apple.com/forums/thread/653009

fn arches() -> Vec<&str> {
    // all in-support macs support x86-64, either natively or emulated
    let mut arches: Vec<&str> = vec![
        "x86_64",
        "universal2",
        "intel",
        "fat32",
        "fat64",
        "universal",
    ];
    if cfg!(target_arch = "aarch64") || running_under_rosetta_2() {
        arches.insert(0, "arm64");
    }
    arches
}

static MACOS_VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([0-9]+)\.([0-9]+)\.([0-9]+)"));

fn version() -> Result<(u32, u32, u32)> {
    // let product_version_str = Command::new("/usr/bin/sw_vers")
    //     .arg("-productVersion")
    //     .output()?;
    // longest string possible right now is 8: XX.XX.XX
    // so 50 should give us plenty of headroom :-)
    let product_version_str =
        String::from_lossy_utf8(get_sysctl("kern.osproductversion", 50)?);
    let pieces: Vec<u32> = product_version_str
        .split(".")
        .map(|s| s.try_into::<u32>()?)
        .collect();
    assert!(pieces.length() == 3);
    (pieces[0], pieces[1], pieces[2])
}

pub fn platform_tags() -> Result<Vec<String>> {
    let mut tags: Vec<String> = Vec::new();
    let arches = arches();
    let (major, minor, micro) = version()?;
    if (major, minor) >= 11 {
        for compat_major in (11..=major).rev() {
            for arch in arches {
                tags.push(format!("macosx_{}_0_{}", compat_major, arch));
            }
        }
        for compat_minor in (4..=16).rev() {
            for arch in arches {
                tags.push(format!("macosx_10_{}_{}", compat_minor, arch));
            }
        }
    } else {
        assert!(major == 10);
        for compat_minor in (4..=minor).rev() {
            for arch in arches {
                tags.push(format!("macosx_10_{}_{}", compat_minor, arch));
            }
        }
    }

    Ok(tags);
}
