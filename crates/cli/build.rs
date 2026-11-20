// SPDX-License-Identifier: FSL-1.1-ALv2
fn main() {
    // Stamp the target triple into `hn version` (per-OS builds for the plugins, D-046).
    println!(
        "cargo:rustc-env=HN_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
