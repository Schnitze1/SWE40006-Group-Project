fn main() {
    // gaze-pii → ort-sys (ONNX Runtime) is C++ and needs libstdc++ on Linux
    // when cargo-lambda cross-links aarch64 with lld/zig.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
