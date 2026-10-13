# jvm-instrument

jvm-instrument helps you run Rust code inside a Java program that is already running. Your code gets `java.lang.instrument` without the `-javaagent` flag, the attach API or an agent jar.

Build your code as a `cdylib` and load it into the JVM's process with an injector, `dlopen` or `LoadLibrary` and call `jvm_instrument::spawn` from the library's constructor. On a new thread, `spawn` attaches to the running JVM and hands you an `Instrumentation`, letting you add `ClassFileTransformer`s, retransform and redefine loaded classes, and add jars to the bootstrap or system class loader's search.

## Quickstart

```rust
#[ctor::ctor]
unsafe fn load() {
    jvm_instrument::spawn(|env, instrumentation| {
        // Add a ClassFileTransformer, retransform classes, register natives, …
        Ok(())
    });
}
```

## Doing it yourself

`spawn` is a few lines over two functions: `java_vm`, which finds the running JVM, and `create_instrumentation`. Call them yourself to do something between the steps, or to run on a thread of your own:

```rust
#[ctor::ctor]
unsafe fn load() {
    // On Windows a constructor runs inside DllMain, which holds the loader
    // lock. Attaching to the JVM there would deadlock, so use a new thread.
    std::thread::spawn(|| {
        let Some(vm) = jvm_instrument::java_vm() else {
            return;
        };
        // Attaching gives this thread a JNI environment.
        let result = vm.attach_current_thread(|env| -> Result<(), Box<dyn std::error::Error>> {
            // Call this once per JVM. Each call allocates memory that is never
            // freed.
            let instrumentation = jvm_instrument::create_instrumentation(env)?;
            // Your code.
            Ok(())
        });
        if let Err(e) = result {
            eprintln!("{e}");
        }
    });
}
```

## Support

- **OpenJDK-based JVMs (HotSpot), Java 8 and newer.** jvm-instrument depends on OpenJDK internals, such as the `JPLISAgent` struct's layout. It has been tested against Java 8, 17, 21, and 25 on MacOS, but should work on any platform and all versions Java 8+.
