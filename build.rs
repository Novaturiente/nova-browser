fn main() {
    // libcef.so and CEF's resources sit next to the binary (cef-dll-sys copies them
    // into target/, the package installs them in /usr/lib/novabrowser).
    println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
}
