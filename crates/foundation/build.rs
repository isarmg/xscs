fn main() {
    println!("cargo:rerun-if-env-changed=TARGET");
    let target = std::env::var("TARGET").expect("Cargo provides TARGET");
    assert_eq!(
        target, "x86_64-unknown-linux-gnu",
        "xcss supports only x86_64-unknown-linux-gnu server builds"
    );
}
