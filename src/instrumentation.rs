//! Builds a `java.lang.instrument.Instrumentation` without a Java agent.
//!
//! For a real agent, `libinstrument` fills in a JPLIS agent struct and
//! constructs OpenJDK's `sun.instrument.InstrumentationImpl` around it. This
//! module does the same by hand. It depends on OpenJDK internals. The
//! `JPLISAgent` layout is copied from OpenJDK, and the signatures of
//! `InstrumentationImpl`'s constructor and `transform` method differ between
//! Java versions.
use core::ffi::c_char;

use jni::refs::Global;
use jni::sys::{JavaVM, jboolean, jmethodID, jobject};
use jni::{JValue, jni_sig, jni_str};
use std::ffi::c_void;
use std::ptr;

use crate::jvmti::{self, Capabilities};
use crate::{Error, jvm::Instrumentation};

#[repr(C)]
struct JPLISEnvironment {
    pub m_jvmtienv: jvmti::Env,
    pub m_agent: *mut JPLISAgent, // the agent this environment belongs to
    pub m_is_retransformer: jboolean,
}

#[repr(C)]
struct JPLISAgent {
    pub m_jvm: *mut JavaVM,
    pub m_normal_environment: JPLISEnvironment,
    pub m_retransform_environment: JPLISEnvironment,
    pub m_instrumentation_impl: jobject,
    pub m_premain_caller: jmethodID,
    pub m_agentmain_caller: jmethodID,
    pub m_transform: jmethodID,
    pub m_redefine_available: jboolean,
    pub m_redefine_added: jboolean,
    pub m_native_method_prefix_available: jboolean,
    pub m_native_method_prefix_added: jboolean,
    pub m_agent_class_name: *const c_char, // char const*
    pub m_options_string: *const c_char,   // char const*
    pub m_jarfile: *const c_char,          // const char*
    pub m_print_warning: jboolean,
}

/// Makes a JPLIS agent for `vm`, with `env` as its normal environment. It's
/// never freed.
unsafe fn create_dummy_agent(vm: *mut JavaVM, env: jvmti::Env) -> Result<*mut JPLISAgent, Error> {
    let agent = Box::into_raw(Box::new(JPLISAgent {
        m_jvm: vm,
        m_normal_environment: JPLISEnvironment {
            m_jvmtienv: env,
            m_agent: ptr::null_mut(),
            m_is_retransformer: false,
        },
        // libinstrument makes this environment when it's first needed.
        m_retransform_environment: JPLISEnvironment {
            m_jvmtienv: ptr::null_mut(),
            m_agent: ptr::null_mut(),
            m_is_retransformer: true,
        },
        m_instrumentation_impl: ptr::null_mut(),
        m_premain_caller: ptr::null_mut(),
        m_agentmain_caller: ptr::null_mut(),
        m_transform: ptr::null_mut(),
        m_redefine_available: false,
        m_redefine_added: true,
        m_native_method_prefix_available: false,
        // libinstrument only adds can_set_native_method_prefix to the
        // environments it makes when this is set.
        m_native_method_prefix_added: true,
        m_agent_class_name: ptr::null(),
        m_options_string: ptr::null(),
        m_jarfile: ptr::null(),
        m_print_warning: false,
    }));
    unsafe {
        (*agent).m_normal_environment.m_agent = agent;
        (*agent).m_retransform_environment.m_agent = agent;
        jvmti::set_environment_local_storage(env, &raw const (*agent).m_normal_environment as *const c_void)?;
    }
    Ok(agent)
}

/// Creates an `Instrumentation` for the JVM that `env` belongs to. It can
/// retransform and redefine classes.
///
/// Call it once per JVM, from a thread attached to it. Each call makes new
/// JVMTI environments and a new agent, and none are ever freed.
pub fn create_instrumentation(env: &mut jni::Env) -> Result<Global<Instrumentation<'static>>, Error> {
    let vm = env.get_java_vm()?.get_raw();
    unsafe {
        let mut jvmti_env: jvmti::Env = ptr::null_mut();
        let rc = ((*(*vm)).v1_2.GetEnv)(
            vm,
            (&raw mut jvmti_env).cast::<*mut c_void>(),
            jvmti::JVMTI_VERSION_1_2,
        );
        if rc != jni::sys::JNI_OK || jvmti_env.is_null() {
            return Err(Error::Jvmti(format!("GetEnv(JVMTI_VERSION_1_2) failed: {rc}")));
        }

        // First, so a JVM that can't retransform fails before anything else is
        // allocated.
        jvmti::add_capabilities(
            jvmti_env,
            &Capabilities::with(&[
                Capabilities::CAN_REDEFINE_CLASSES,
                Capabilities::CAN_SET_NATIVE_METHOD_PREFIX,
                Capabilities::CAN_RETRANSFORM_CLASSES,
                Capabilities::CAN_RETRANSFORM_ANY_CLASS,
            ]),
        )?;
        let agent = create_dummy_agent(vm, jvmti_env)?;
        let inst_class = env.find_class(jni_str!("sun/instrument/InstrumentationImpl"))?;
        let agent_ptr: jni::sys::jlong = agent as isize as jni::sys::jlong;
        // The arguments are the agent, whether redefinition is available and
        // whether native method prefixes are available. Java 21 added
        // printWarning, which is false so the JVM prints no warning about
        // agents.
        let inst = match env.new_object(
            &inst_class,
            jni_sig!("(JZZZ)V"),
            &[JValue::Long(agent_ptr), JValue::Bool(true), JValue::Bool(true), JValue::Bool(false)],
        ) {
            Ok(inst) => inst,
            Err(_) => {
                env.exception_clear();
                env.new_object(
                    &inst_class,
                    jni_sig!("(JZZ)V"),
                    &[JValue::Long(agent_ptr), JValue::Bool(true), JValue::Bool(true)],
                )?
            }
        };
        let inst_ref = env.new_global_ref(&inst)?;
        // Java 9 added the module parameter.
        let transform_method = match env.get_method_id(&inst_class, jni_str!("transform"), jni_sig!(
            "(Ljava/lang/Module;Ljava/lang/ClassLoader;Ljava/lang/String;Ljava/lang/Class;Ljava/security/ProtectionDomain;[BZ)[B"
        )) {
            Ok(id) => id,
            Err(_) => {
                env.exception_clear();
                env.get_method_id(&inst_class, jni_str!("transform"), jni_sig!(
                    "(Ljava/lang/ClassLoader;Ljava/lang/String;Ljava/lang/Class;Ljava/security/ProtectionDomain;[BZ)[B"
                ))?
            }
        };

        // The JVM calls back through the agent on every retransform, long
        // after this thread's local frame ends. So the agent gets its own
        // global reference, which is leaked like the agent itself.
        let agent_inst = env.new_global_ref(&inst)?;
        (*agent).m_instrumentation_impl = agent_inst.as_raw();
        std::mem::forget(agent_inst);
        (*agent).m_transform = transform_method.into_raw();

        // libinstrument calls transformers from the ClassFileLoadHook handler
        // it registers on the environments it makes itself. That handler is
        // private, and for a real agent libinstrument makes the normal
        // environment too. It makes the retransforming environment on demand,
        // so have it make one and use that as the normal environment instead.
        // It makes another retransforming environment when one is needed.
        env.call_method(
            &inst,
            jni_str!("setHasRetransformableTransformers"),
            jni_sig!("(JZ)V"),
            &[JValue::Long(agent_ptr), JValue::Bool(false)],
        )?;
        let normal_env = (*agent).m_retransform_environment.m_jvmtienv;
        if normal_env.is_null() {
            return Err(Error::Jvmti("libinstrument made no JVMTI environment".to_owned()));
        }
        (*agent).m_retransform_environment.m_jvmtienv = ptr::null_mut();
        // An environment that has can_retransform_classes when its
        // ClassFileLoadHook is first enabled is called on retransforms too.
        // Plain transformers must not be, so drop it before then.
        jvmti::relinquish_capabilities(normal_env, &Capabilities::with(&[Capabilities::CAN_RETRANSFORM_CLASSES]))?;
        // libinstrument doesn't give it can_redefine_classes, which
        // redefineClasses needs on the normal environment.
        jvmti::add_capabilities(normal_env, &Capabilities::with(&[Capabilities::CAN_REDEFINE_CLASSES]))?;
        (*agent).m_normal_environment.m_jvmtienv = normal_env;
        jvmti::set_environment_local_storage(normal_env, &raw const (*agent).m_normal_environment as *const c_void)?;
        // Newer Javas enable the normal environment's ClassFileLoadHook when
        // the first plain transformer is added, through setHasTransformers.
        // Older ones expect libinstrument to have enabled it at startup.
        if env.get_method_id(&inst_class, jni_str!("setHasTransformers"), jni_sig!("(JZ)V")).is_err() {
            env.exception_clear();
            jvmti::set_event_notification_mode(
                normal_env,
                jvmti::JVMTI_ENABLE,
                jvmti::JVMTI_EVENT_CLASS_FILE_LOAD_HOOK,
            )?;
        }

        // JVMTI calls environments in the order they were made. A real agent
        // that can retransform makes its retransforming environment at
        // startup, so make it now too, not when the first retransformable
        // transformer is added. Its ClassFileLoadHook stays off until then.
        env.call_method(
            &inst,
            jni_str!("setHasRetransformableTransformers"),
            jni_sig!("(JZ)V"),
            &[JValue::Long(agent_ptr), JValue::Bool(false)],
        )?;

        Ok(env.cast_global::<Instrumentation>(inst_ref)?)
    }
}
