//! Bindings for `java.lang.instrument`. `JarFile` is here because
//! `Instrumentation` takes one to add a jar to a class loader's search.

use jni::bind_java_type;

bind_java_type! { pub ClassFileTransformer => java.lang.instrument.ClassFileTransformer }
bind_java_type! { pub ClassDefinition => java.lang.instrument.ClassDefinition }
bind_java_type! {
    pub JarFile => java.util.jar.JarFile,

    constructors {
        fn new(path: JString),
    }
}

bind_java_type! {
    pub Instrumentation => java.lang.instrument.Instrumentation,

    type_map = {
        ClassFileTransformer => java.lang.instrument.ClassFileTransformer,
        ClassDefinition => java.lang.instrument.ClassDefinition,
        JarFile => java.util.jar.JarFile,
    },

    methods {
        fn add_transformer {
            name = "addTransformer",
            sig = (transformer: ClassFileTransformer),
        },

        fn add_transformer_with_retransform {
            name = "addTransformer",
            sig = (transformer: ClassFileTransformer, can_retransform: jboolean),
        },

        fn remove_transformer(transformer: ClassFileTransformer) -> jboolean,

        fn is_retransform_classes_supported() -> jboolean,
        fn retransform_classes(classes: JClass[]),

        fn is_redefine_classes_supported() -> jboolean,
        fn redefine_classes(definitions: ClassDefinition[]),

        fn is_modifiable_class(the_class: JClass) -> jboolean,

        fn get_all_loaded_classes() -> JClass[],
        fn get_initiated_classes(loader: JClassLoader) -> JClass[],

        fn get_object_size(object_to_size: JObject) -> jlong,

        fn append_to_bootstrap_class_loader_search(jarfile: JarFile),
        fn append_to_system_class_loader_search(jarfile: JarFile),

        fn is_native_method_prefix_supported() -> jboolean,
        fn set_native_method_prefix(transformer: ClassFileTransformer, prefix: JString),
    }
}
