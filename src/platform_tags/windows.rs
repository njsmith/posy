use crate::prelude::*;

// https://docs.microsoft.com/en-us/windows/win32/sysinfo/image-file-machine-constants
const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

#[link(name = "kernel32")]
#[allow(non_snake_case)]
extern "system" {
    #[must_use]
    fn IsWow64GuestMachineSupported(machine: u16, out: *mut u8) -> u32;
}

fn is_wow64_guest_machine_supported(machine: u16) -> Result<bool> {
    let mut out: u8 = 0;
    unsafe {
        let hresult = IsWow64GuestMachineSupported(machine, u8.as_mut_ptr());
    }
    if hresult {
        Err(std::error::last_os_error())
    } else {
        Ok(out as bool)
    }
}

pub fn platform_tags() -> Result<Vec<String>> {
    let mut tags: Vec<String> = vec![];
    if cfg!(target_arch = "x86_64")
        || is_wow64_guest_machine_supported(IMAGE_FILE_MACHINE_AMD64)?
    {
        tags.push("win_amd64");
    }
    if cfg!(target_arch = "x86")
        || is_wow64_guest_machine_supported(IMAGE_FILE_MACHINE_I386)?
    {
        tags.push("win32");
    }
    Ok(tags)
}
