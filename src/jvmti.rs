//! The parts of JVMTI this crate uses, declared from OpenJDK's `jvmti.h`.
//!
//! A `jvmtiEnv*` points to a pointer to JVMTI's function table. Functions are
//! looked up by their number in `jvmti.h`, which never changes between Java
//! versions.

use std::ffi::c_void;

use jni::sys::{jint, jobject};

use crate::Error;

pub(crate) const JVMTI_VERSION_1_2: jint = 0x3001_0200;
pub(crate) const JVMTI_ENABLE: jint = 1;
pub(crate) const JVMTI_EVENT_CLASS_FILE_LOAD_HOOK: jint = 54;

/// `jvmtiEnv*`.
pub(crate) type Env = *mut *const *const c_void;

/// `jvmtiCapabilities`: 128 one-bit fields, numbered as in `jvmti.h`.
#[repr(C)]
#[derive(Default)]
pub(crate) struct Capabilities([u32; 4]);

impl Capabilities {
    pub(crate) const CAN_REDEFINE_CLASSES: u32 = 9;
    pub(crate) const CAN_SET_NATIVE_METHOD_PREFIX: u32 = 36;
    pub(crate) const CAN_RETRANSFORM_CLASSES: u32 = 37;
    pub(crate) const CAN_RETRANSFORM_ANY_CLASS: u32 = 38;

    pub(crate) fn with(capabilities: &[u32]) -> Self {
        let mut caps = Self::default();
        for &n in capabilities {
            // C compilers fill bitfields from the low bit on little-endian
            // targets and from the high bit on big-endian ones.
            let bit = if cfg!(target_endian = "big") { 31 - n % 32 } else { n % 32 };
            caps.0[(n / 32) as usize] |= 1 << bit;
        }
        caps
    }
}

/// Function `number` of `env`'s function table.
unsafe fn function(env: Env, number: usize) -> *const c_void {
    unsafe { *(*env).add(number - 1) }
}

fn check(name: &str, err: jint) -> Result<(), Error> {
    if err == 0 {
        Ok(())
    } else {
        Err(Error::Jvmti(format!("{name} failed: jvmtiError {err}")))
    }
}

pub(crate) unsafe fn add_capabilities(env: Env, caps: &Capabilities) -> Result<(), Error> {
    type F = unsafe extern "system" fn(Env, *const Capabilities) -> jint;
    unsafe {
        let f = std::mem::transmute::<*const c_void, F>(function(env, 142));
        check("AddCapabilities", f(env, caps))
    }
}

pub(crate) unsafe fn relinquish_capabilities(env: Env, caps: &Capabilities) -> Result<(), Error> {
    type F = unsafe extern "system" fn(Env, *const Capabilities) -> jint;
    unsafe {
        let f = std::mem::transmute::<*const c_void, F>(function(env, 143));
        check("RelinquishCapabilities", f(env, caps))
    }
}

pub(crate) unsafe fn set_environment_local_storage(env: Env, data: *const c_void) -> Result<(), Error> {
    type F = unsafe extern "system" fn(Env, *const c_void) -> jint;
    unsafe {
        let f = std::mem::transmute::<*const c_void, F>(function(env, 148));
        check("SetEnvironmentLocalStorage", f(env, data))
    }
}

/// Sets `event`'s mode for all threads.
pub(crate) unsafe fn set_event_notification_mode(env: Env, mode: jint, event: jint) -> Result<(), Error> {
    // It's variadic, so it uses the C calling convention on every platform.
    type F = unsafe extern "C" fn(Env, jint, jint, jobject, ...) -> jint;
    unsafe {
        let f = std::mem::transmute::<*const c_void, F>(function(env, 2));
        check("SetEventNotificationMode", f(env, mode, event, std::ptr::null_mut()))
    }
}
