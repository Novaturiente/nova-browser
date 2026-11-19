fn main() {
    // libcef.so sits next to the binary (cef-dll-sys copies it into target/)
    println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
}
