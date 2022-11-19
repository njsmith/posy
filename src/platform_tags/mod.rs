/// Utilities to introspect the system that posy is running on, and figure out which
/// .pybi's it will be able to run. This is similar to what 'packaging.tags' does, but
/// with two key differences:
///
/// - We only care about "platform" tags (like "win32"), not the python-specific tags
///   (like "cp37")
///
/// - For 'packaging.tags', the question is "what wheels can run on this interpreter
///   that I've already installed?" So like, if it's running on a 64-bit interpreter, it
///   automatically knows that it only needs to consider 64-bit wheels.
///
///   Our question is a bit different: "what interpreters could I install on this
///   system?" So even if posy itself is built for 64-bit, we might still be able to run
///   a 32-bit interpreter, or vice-versa. Recent Mac's can run both arm64 and x86-64
///   interpreters. And so on. So we can't just check how we were built and be done --
///   we have to poke around the system much more.

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::core_platform_tags;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::core_platform_tags;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::core_platform_tags;

mod expand;
pub use expand::{
    current_platform_tags, expand_platform_tag, Platform, PybiPlatform, WheelPlatform,
};
