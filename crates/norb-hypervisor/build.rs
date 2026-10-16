fn main() {
    // Link against Apple's Hypervisor.framework.
    // This framework is only available on macOS 11+ on Apple Silicon and
    // macOS 10.15+ on Intel. The cargo cfg gate in Cargo.toml handles
    // platform selection; here we unconditionally emit the linker flag so
    // that `cargo check` on non-macOS platforms doesn't fail at *link* time
    // (it will fail at *compile* time via `cfg` guards in the source instead).
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=Hypervisor");
}
