use crate::prelude::*;

// https://docs.microsoft.com/en-us/windows/win32/sysinfo/image-file-machine-constants
const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;

#[link(name = "kernel32")]
#[allow(non_snake_case)]
extern "system" {
    // This tells us which *non-native* platforms the system can run (e.g., x86-32 when
    // the system is x86-64, or x86-64 if the system is arm64). But it doesn't tell us
    // what the native platform is. For that we need...
    #[must_use]
    fn IsWow64GuestMachineSupported(machine: u16, out: *mut u8) -> u32;

    // ...this function. Together, we can get the full list of supported platforms.
    #[must_use]
    fn IsWow64Process2(
        hProcess: *const std::ffi::c_void,
        _process_type: *mut u16,
        system_type: *mut u16,
    ) -> u32;
}

fn is_wow64_guest_machine_supported(machine: u16) -> Result<bool> {
    let mut out: u8 = 0;
    let hresult =
        unsafe { IsWow64GuestMachineSupported(machine, (&mut out) as *mut u8) };
    if hresult != 0 {
        Err(std::io::Error::last_os_error())?
    } else {
        Ok(out != 0)
    }
}

fn system_type() -> Result<u16> {
    let mut _process_type: u16 = 0;
    let mut system_type: u16 = 0;
    let result = unsafe {
        IsWow64Process2(
            // the magic handle -1 means "the current process"
            -1 as *const std::ffi::c_void,
            &mut _process_type as *mut u16,
            &mut system_type as *mut u16,
        )
    };
    if result != 0 {
        Err(std::io::Error::last_os_error())?
    } else {
        Ok(system_type)
    }
}

const MACHINES: &[u16] = &[
    IMAGE_FILE_MACHINE_I386,
    IMAGE_FILE_MACHINE_AMD64,
    IMAGE_FILE_MACHINE_ARM64,
];

fn map(machine: u16) -> Result<&'static str> {
    match machine {
        IMAGE_FILE_MACHINE_I386 => "win32",
        IMAGE_FILE_MACHINE_AMD64 => "win_amd64",
        IMAGE_FILE_MACHINE_ARM64 => "win_arm64",
        _ => bail!("unknown machine constant {:#x}", machine),
    }
}

pub fn core_platform_tags() -> Result<Vec<String>> {
    let mut tags: Vec<String> = vec![];

    let native = system_type()?;
    tags.push(map(native)?);

    for machine in MACHINES {
        if machine != native && is_wow64_guest_machine_supported(machine)? {
            tags.push(map(machine)?);
        }
    }

    Ok(tags)
}
