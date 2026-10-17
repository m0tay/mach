fn main() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rerun-if-changed=src/vz.m");
        println!("cargo:rerun-if-changed=build.rs");

        cc::Build::new()
            .file("src/vz.m")
            .compile("mach_vz");

        println!("cargo:rustc-link-lib=framework=Virtualization");
        println!("cargo:rustc-link-lib=framework=Foundation");
    }
}
