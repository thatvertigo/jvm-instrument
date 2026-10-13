//! Gives Rust code loaded into a running JVM `java.lang.instrument`, without a
//! Java agent.
//!
//! Build your library as a `cdylib` and load it into the JVM's process with an
//! injector, `dlopen` or `LoadLibrary`. From its constructor, call [`spawn`].
//! On a new thread, it attaches to the JVM, builds an `Instrumentation` and
//! calls your closure with both:
//!
//! ```ignore
//! #[ctor::ctor]
//! unsafe fn load() {
//!     jvm_instrument::spawn(|env, instrumentation| {
//!         // Add a ClassFileTransformer, retransform classes, register natives, …
//!         Ok(())
//!     });
//! }
//! ```
//!
//! Put the constructor in the `cdylib` itself. The linker can drop a
//! constructor that lives in a dependency, so this crate doesn't provide one.
//!
//! [`spawn`] is a few lines over the two functions that do the work. Call them
//! yourself to do the steps your own way:
//! - [`java_vm`] finds the JVM this process runs, on macOS, Linux and Windows.
//!   The host's library doesn't link against the JVM library.
//! - [`create_instrumentation`] builds the `Instrumentation`.
//!
//! # Requirements
//! An OpenJDK-based JVM (HotSpot), Java 8 or newer, because
//! [`create_instrumentation`] depends on OpenJDK internals.

use std::{fmt, thread::JoinHandle};

use jni::{Env, JavaVM, refs::Global};

mod instrumentation;
mod jvmti;
pub mod jvm;
mod vm;

pub use instrumentation::create_instrumentation;
/// The `jni` crate this crate's API uses. Use it to get matching types.
pub use jni;

use crate::jvm::Instrumentation;

/// The JVM this process runs, if one has started.
///
/// It looks `JNI_GetCreatedJavaVMs` up in the JVM library the process has
/// already loaded (`libjvm.dylib`, `libjvm.so` or `jvm.dll`), so the host's
/// library doesn't link against it.
pub fn java_vm() -> Option<JavaVM> {
    vm::created_vm().map(|vm| unsafe { JavaVM::from_raw(vm) })
}

/// An error from the code [`spawn`] runs.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Runs `f` on a new thread attached to the running JVM, with an
/// `Instrumentation`. Call it from your library's constructor.
///
/// It uses a new thread because on Windows a constructor runs inside
/// `DllMain`, which holds the loader lock. Attaching to the JVM there would
/// deadlock.
///
/// If finding the JVM, building the `Instrumentation` or `f` fails, `spawn`
/// logs the error through the [`log`] crate, because nothing waits on a
/// thread started from a constructor. The handle returns the error too.
pub fn spawn<F>(f: F) -> JoinHandle<Result<(), Error>>
where
    F: FnOnce(&mut Env, Global<Instrumentation<'static>>) -> Result<(), BoxError> + Send + 'static,
{
    std::thread::spawn(move || {
        let result = attach(f);
        if let Err(e) = &result {
            log::error!("{e}");
        }
        result
    })
}

fn attach<F>(f: F) -> Result<(), Error>
where
    F: FnOnce(&mut Env, Global<Instrumentation<'static>>) -> Result<(), BoxError>,
{
    let vm = java_vm().ok_or(Error::NoJvm)?;
    vm.attach_current_thread(|env| {
        let instrumentation = create_instrumentation(env)?;
        f(env, instrumentation).map_err(Error::Run)
    })
}

/// Why getting an `Instrumentation`, or the code [`spawn`] ran, failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// No JVM is running in this process.
    NoJvm,
    /// A JNI call failed.
    Jni(jni::errors::Error),
    /// JVMTI wasn't available, or a JVMTI call failed.
    Jvmti(String),
    /// The code [`spawn`] ran failed.
    Run(BoxError),
}

impl From<jni::errors::Error> for Error {
    fn from(e: jni::errors::Error) -> Self {
        Error::Jni(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoJvm => f.write_str("no JVM is running in this process"),
            Error::Jni(e) => write!(f, "JNI: {e}"),
            Error::Jvmti(e) => write!(f, "JVMTI: {e}"),
            Error::Run(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Jni(e) => Some(e),
            Error::Run(e) => Some(e.as_ref()),
            Error::NoJvm | Error::Jvmti(_) => None,
        }
    }
}
