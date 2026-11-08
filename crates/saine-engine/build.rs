//! Builds the C++ engine with CMake (Ninja) and links it statically.
fn main() {
    let engine = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../engine");
    println!("cargo:rerun-if-changed={}", engine.join("include").display());
    println!("cargo:rerun-if-changed={}", engine.join("src").display());
    println!("cargo:rerun-if-changed={}", engine.join("CMakeLists.txt").display());
    let profile = if std::env::var("PROFILE").as_deref() == Ok("release") { "Release" } else { "RelWithDebInfo" };
    let dst = cmake::Config::new(&engine)
        .generator("Ninja")
        .profile(profile)
        .define("SAINE_ENGINE_BUILD_TESTS", "OFF")
        .define("SAINE_ENGINE_BUILD_BENCH", "OFF")
        .define("SAINE_ENGINE_BUILD_TOOLS", "OFF")
        .build_target("saine_engine")
        .build();
    // The static library lands in the CMake build tree.
    println!("cargo:rustc-link-search=native={}/build", dst.display());
    println!("cargo:rustc-link-lib=static=saine_engine");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=c++");
            for fw in ["CoreAudio", "AudioToolbox", "AudioUnit", "CoreFoundation", "Accelerate"] {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        }
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }
}
