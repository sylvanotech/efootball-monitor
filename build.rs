fn main() {
    // On Windows/MSVC, delay-load wpcap.dll so the program can locate Npcap's
    // directory at runtime (and print a helpful error when Npcap is missing)
    // instead of failing to start. Test binaries never touch pcap, so they also
    // run on machines without Npcap.
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && env == "msvc" {
        println!("cargo:rustc-link-arg=/DELAYLOAD:wpcap.dll");
        println!("cargo:rustc-link-lib=delayimp");
    }
}
