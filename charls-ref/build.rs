//! Builds the CharLS sources of the `charls` submodule as a static library.

fn main() {
    println!("cargo:rerun-if-changed=charls/CMakeLists.txt");
    println!("cargo:rerun-if-changed=charls/src");
    if !std::path::Path::new("charls/CMakeLists.txt").exists() {
        panic!(
            "charls-ref/charls is empty: the CharLS sources are a git submodule. \
             Run `git submodule update --init`."
        );
    }
    let built = cmake::Config::new("charls")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CHARLS_BUILD_TESTS", "OFF")
        .define("CHARLS_BUILD_FUZZ_TEST", "OFF")
        .define("CHARLS_BUILD_SAMPLES", "OFF")
        // optimized whatever the profile: the tests push thousands of images through it
        .profile("Release")
        .build();
    println!("cargo:rustc-link-search=native={}/lib", built.display());
    println!("cargo:rustc-link-search=native={}/lib64", built.display());
    println!("cargo:rustc-link-lib=static=charls");
}
