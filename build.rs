use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=SPARKLE_FRAMEWORK_DIR");
    println!("cargo:rerun-if-changed=scripts/sparkle-poc/bridge.m");
    println!("cargo:rerun-if-changed=native/updater.m");
    let production = env::var_os("CARGO_FEATURE_SPARKLE").is_some();
    let poc = env::var_os("CARGO_FEATURE_SPARKLE_POC").is_some();
    assert!(
        !(production && poc),
        "sparkle and sparkle-poc are mutually exclusive"
    );
    if !production && !poc {
        return;
    }
    let source = if production {
        "native/updater.m"
    } else {
        "scripts/sparkle-poc/bridge.m"
    };
    let framework =
        env::var("SPARKLE_FRAMEWORK_DIR").expect("set SPARKLE_FRAMEWORK_DIR for Sparkle builds");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("updater.o");
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture: {other}"),
    };
    assert!(Command::new("clang")
        .args([
            "-fobjc-arc",
            "-arch",
            arch,
            "-mmacosx-version-min=14.0",
            "-F",
            &framework,
            "-c",
            source,
            "-o"
        ])
        .arg(&object)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("ar")
        .arg("rcs")
        .arg(out.join("libsparkle_poc.a"))
        .arg(object)
        .status()
        .unwrap()
        .success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=sparkle_poc");
    println!("cargo:rustc-link-search=framework={framework}");
    println!("cargo:rustc-link-lib=framework=Sparkle");
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
}
