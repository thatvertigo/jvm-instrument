//! Writes the bytes of every class the JVM loads to disk.
//!
//! Build it with `cargo build --example classdumper` and load the library
//! into a running JVM. It writes each class to
//! `<dir>/<package>/<Name>.class`, where `<dir>` is `$CLASSDUMP_DIR`, or
//! `classdump` in the JVM's working directory. It dumps the classes loaded
//! before it was, then every class loaded after.
//!
//! A `ClassFileTransformer` has to be a Java object, so this defines a small
//! class whose `transform` method is native, and implements that in Rust.
//!
//! Classes that were loaded already are dumped by retransforming them. The
//! JVM rebuilds their bytes from what it holds in memory, so they're
//! equivalent to the original class file but not identical to it. Classes
//! loaded later are dumped as they were read.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use jvm_instrument::jni::{self, Env, NativeMethod, native_method};
use jni::errors::LogErrorAndDefault;
use jni::objects::{JByteArray, JClass, JClassLoader, JObject, JObjectArray, JString};
use jni::refs::{Global, IntoAuto};
use jni::strings::JNIStr;
use jvm_instrument::jvm::{ClassFileTransformer, Instrumentation};

static DUMP_DIR: OnceLock<PathBuf> = OnceLock::new();
static DUMPED: AtomicUsize = AtomicUsize::new(0);

#[ctor::ctor]
unsafe fn load() {
    env_logger::init();
    jvm_instrument::spawn(dump_classes);
}

fn dump_classes(env: &mut Env, instrumentation: Global<Instrumentation<'static>>) -> Result<(), jvm_instrument::BoxError> {
    let dir = DUMP_DIR.get_or_init(|| std::env::var_os("CLASSDUMP_DIR").unwrap_or("classdump".into()).into());
    log::info!("dumping classes to {}", dir.display());

    // The bootstrap class loader can see ClassFileTransformer on every Java
    // version. The name comes from the class file.
    let class = env.define_class(None::<&JNIStr>, JClassLoader::null(), &transformer_class())?;
    unsafe { env.register_native_methods(&class, &[TRANSFORM])? };
    // The class has no constructor, so make the object without running one.
    let transformer = env.alloc_object(&class)?;
    let transformer = env.cast_local::<ClassFileTransformer>(transformer)?;
    instrumentation.add_transformer_with_retransform(env, &transformer, true)?;

    // retransformClasses fails if any class in the array can't be modified,
    // such as arrays, primitives and hidden classes.
    let loaded = instrumentation.get_all_loaded_classes(env)?;
    let mut modifiable = Vec::new();
    for i in 0..loaded.len(env)? {
        let class = loaded.get_element(env, i)?.auto();
        if instrumentation.is_modifiable_class(env, &class)? {
            modifiable.push(i);
        }
    }
    let classes = JObjectArray::<JClass>::new(env, modifiable.len(), JClass::null())?;
    for (i, &index) in modifiable.iter().enumerate() {
        let class = loaded.get_element(env, index)?.auto();
        classes.set_element(env, i, &class)?;
    }
    instrumentation.retransform_classes(env, &classes)?;

    log::info!(
        "dumped {} loaded classes to {}, dumping new ones as they load",
        DUMPED.load(Ordering::Relaxed),
        dir.display()
    );
    Ok(())
}

const TRANSFORM: NativeMethod = native_method! {
    jni = jvm_instrument::jni,
    fn transform(
        loader: JClassLoader,
        class_name: JString,
        class_being_redefined: JClass,
        protection_domain: java.security.ProtectionDomain,
        class_file_buffer: jbyte[],
    ) -> jbyte[],
    // An exception thrown from a transformer is silently dropped, so log
    // errors instead.
    error_policy = LogErrorAndDefault,
};

/// Called for each class the JVM loads or retransforms. Returning null
/// leaves the class as it is.
fn transform<'local>(
    env: &mut Env<'local>,
    _this: JObject<'local>,
    _loader: JClassLoader<'local>,
    class_name: JString<'local>,
    _class_being_redefined: JClass<'local>,
    _protection_domain: JObject<'local>,
    class_file_buffer: JByteArray<'local>,
) -> Result<JByteArray<'local>, jni::errors::Error> {
    // Some classes the JVM makes at runtime have no name.
    if class_name.is_null() {
        return Ok(JByteArray::null());
    }
    let name = class_name.try_to_string(env)?;
    let bytes = env.convert_byte_array(&class_file_buffer)?;
    let dir = DUMP_DIR.get().expect("set before the transformer is added");
    match write_class(dir, &name, &bytes) {
        Ok(()) => {
            DUMPED.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => log::error!("couldn't dump {name}: {e}"),
    }
    Ok(JByteArray::null())
}

fn write_class(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let path = dir.join(format!("{name}.class"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

/// Builds the class file for
///
/// ```java
/// package jvm_instrument.examples;
///
/// public final class ClassDumper implements ClassFileTransformer {
///     public native byte[] transform(ClassLoader loader, String className,
///             Class<?> classBeingRedefined, ProtectionDomain protectionDomain,
///             byte[] classfileBuffer);
/// }
/// ```
///
/// without a constructor. It's Java 8's version of the interface. Newer
/// versions add a `transform` that takes a `Module` too, and its default
/// implementation calls this one.
fn transformer_class() -> Vec<u8> {
    fn utf8(out: &mut Vec<u8>, s: &str) {
        out.push(1); // CONSTANT_Utf8
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    fn class(out: &mut Vec<u8>, name_index: u16) {
        out.push(7); // CONSTANT_Class
        out.extend_from_slice(&name_index.to_be_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0xCAFE_BABE_u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // minor version
    out.extend_from_slice(&52u16.to_be_bytes()); // major version, Java 8

    out.extend_from_slice(&9u16.to_be_bytes()); // constant pool count, plus one
    utf8(&mut out, "jvm_instrument/examples/ClassDumper"); // #1
    class(&mut out, 1); // #2
    utf8(&mut out, "java/lang/Object"); // #3
    class(&mut out, 3); // #4
    utf8(&mut out, "java/lang/instrument/ClassFileTransformer"); // #5
    class(&mut out, 5); // #6
    utf8(&mut out, "transform"); // #7
    utf8(
        &mut out,
        "(Ljava/lang/ClassLoader;Ljava/lang/String;Ljava/lang/Class;Ljava/security/ProtectionDomain;[B)[B",
    ); // #8

    out.extend_from_slice(&0x0031u16.to_be_bytes()); // ACC_PUBLIC | ACC_FINAL | ACC_SUPER
    out.extend_from_slice(&2u16.to_be_bytes()); // this class
    out.extend_from_slice(&4u16.to_be_bytes()); // superclass
    out.extend_from_slice(&1u16.to_be_bytes()); // interfaces
    out.extend_from_slice(&6u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // fields

    out.extend_from_slice(&1u16.to_be_bytes()); // methods
    out.extend_from_slice(&0x0101u16.to_be_bytes()); // ACC_PUBLIC | ACC_NATIVE
    out.extend_from_slice(&7u16.to_be_bytes()); // name
    out.extend_from_slice(&8u16.to_be_bytes()); // descriptor
    out.extend_from_slice(&0u16.to_be_bytes()); // attributes

    out.extend_from_slice(&0u16.to_be_bytes()); // attributes
    out
}
