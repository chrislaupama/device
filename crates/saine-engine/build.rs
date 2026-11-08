//! Builds the C++ engine with CMake (Ninja) and links it statically. Also
//! builds the out-of-process plugin scanner and the test plugin bundles; their
//! locations are baked in as SAINE_PLUGIN_SCANNER_BUILT / SAINE_TEST_PLUGIN_DIR
//! (the scanner path can be overridden at runtime with $SAINE_PLUGIN_SCANNER).
//!
//! The VST 3 SDK and CLAP headers are fetched by CMake at pinned tags into
//! `<target>/saine-third-party` (shared by all profiles; see
//! engine/third_party_licenses.md). Offline builds: set
//! FETCHCONTENT_SOURCE_DIR_SAINE_CLAP / FETCHCONTENT_SOURCE_DIR_SAINE_VST3SDK.
fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = manifest.join("../../engine");
    for sub in ["include", "src", "scanner", "test_plugins", "CMakeLists.txt"] {
        println!("cargo:rerun-if-changed={}", engine.join(sub).display());
    }
    println!("cargo:rerun-if-env-changed=FETCHCONTENT_SOURCE_DIR_SAINE_CLAP");
    println!("cargo:rerun-if-env-changed=FETCHCONTENT_SOURCE_DIR_SAINE_VST3SDK");
    let profile = if std::env::var("PROFILE").as_deref() == Ok("release") { "Release" } else { "RelWithDebInfo" };
    let third_party = manifest.join("../../target/saine-third-party");
    let mut cfg = cmake::Config::new(&engine);
    cfg.generator("Ninja")
        .profile(profile)
        .define("SAINE_ENGINE_BUILD_TESTS", "OFF")
        .define("SAINE_ENGINE_BUILD_BENCH", "OFF")
        .define("SAINE_ENGINE_BUILD_TOOLS", "OFF")
        .define("SAINE_ENGINE_BUILD_SCANNER", "ON")
        .define("SAINE_ENGINE_BUILD_TEST_PLUGINS", "ON")
        .define("FETCHCONTENT_BASE_DIR", third_party.to_string_lossy().as_ref())
        .build_target("saine_engine_rust");
    for var in ["FETCHCONTENT_SOURCE_DIR_SAINE_CLAP", "FETCHCONTENT_SOURCE_DIR_SAINE_VST3SDK"] {
        if let Ok(v) = std::env::var(var) {
            cfg.define(var, v);
        }
    }
    let dst = cfg.build();
    let build = dst.join("build");
    // The static library lands in the CMake build tree.
    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=static=saine_engine");
    println!("cargo:rustc-env=SAINE_PLUGIN_SCANNER_BUILT={}", build.join("saine_plugin_scanner").display());
    println!("cargo:rustc-env=SAINE_TEST_PLUGIN_DIR={}", build.join("test_plugins").display());
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=c++");
            println!("cargo:rustc-link-lib=objc");
            for fw in ["CoreAudio", "AudioToolbox", "AudioUnit", "CoreAudioKit", "CoreFoundation", "Foundation", "AppKit", "Accelerate"] {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        }
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }
}
