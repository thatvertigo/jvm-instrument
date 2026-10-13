//! Finds the JVM this process runs, without linking against the JVM library.
//!
//! Linking against it works differently on each platform, so instead the
//! host's library looks `JNI_GetCreatedJavaVMs` up at runtime, in the JVM
//! library the process has already loaded.

use std::ffi::CStr;

use jni::sys::{self, jint, jsize};

type GetCreatedJavaVms =
    unsafe extern "system" fn(*mut *mut sys::JavaVM, jsize, *mut jsize) -> jint;

const SYMBOL: &CStr = c"JNI_GetCreatedJavaVMs";

/// The JVM this process runs, if one has started.
pub(crate) fn created_vm() -> Option<*mut sys::JavaVM> {
    let get_created_java_vms = find_symbol()?;
    let mut vm = std::ptr::null_mut();
    let mut count: jsize = 0;
    let rc = unsafe { get_created_java_vms(&mut vm, 1, &mut count) };
    (rc == sys::JNI_OK && count > 0 && !vm.is_null()).then_some(vm)
}

/// The names the JVM library may be loaded under.
#[cfg(target_vendor = "apple")]
const JVM_LIBRARIES: &[&CStr] = &[c"@rpath/libjvm.dylib", c"libjvm.dylib"];
#[cfg(all(unix, not(target_vendor = "apple")))]
const JVM_LIBRARIES: &[&CStr] = &[c"libjvm.so"];

#[cfg(unix)]
fn find_symbol() -> Option<GetCreatedJavaVms> {
    // The `java` launcher loads the JVM library globally, where RTLD_DEFAULT
    // finds it. A launcher that loads it locally hides it from RTLD_DEFAULT,
    // so then ask for the library by name. RTLD_NOLOAD only returns a library
    // that is already loaded.
    let mut symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, SYMBOL.as_ptr()) };
    for name in JVM_LIBRARIES {
        if !symbol.is_null() {
            break;
        }
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD) };
        if !handle.is_null() {
            symbol = unsafe { libc::dlsym(handle, SYMBOL.as_ptr()) };
            // dlopen added a reference to the library. The JVM keeps it
            // loaded, so drop ours.
            unsafe { libc::dlclose(handle) };
        }
    }
    (!symbol.is_null())
        .then(|| unsafe { std::mem::transmute::<*mut libc::c_void, GetCreatedJavaVms>(symbol) })
}

#[cfg(windows)]
fn find_symbol() -> Option<GetCreatedJavaVms> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    // GetModuleHandleW only returns a library that is already loaded.
    let name: Vec<u16> = "jvm.dll\0".encode_utf16().collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        return None;
    }
    let proc = unsafe { GetProcAddress(module, SYMBOL.as_ptr().cast()) }?;
    Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, GetCreatedJavaVms>(proc) })
}

#[cfg(not(any(unix, windows)))]
fn find_symbol() -> Option<GetCreatedJavaVms> {
    None
}
