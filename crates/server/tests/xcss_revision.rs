#[test]
fn embedded_foundation_is_bound_to_the_product_source() {
    assert_eq!(env!("XCSS_REVISION"), env!("XSCS_SOURCE_REVISION"));
    let lockfile = include_str!("../../../Cargo.lock");
    let packages = lockfile
        .split("[[package]]")
        .filter(|package| {
            package
                .lines()
                .any(|line| line.trim() == r#"name = "xscs-foundation""#)
        })
        .collect::<Vec<_>>();
    assert_eq!(packages.len(), 1);
    assert!(
        !packages[0]
            .lines()
            .any(|line| line.trim().starts_with("source ="))
    );
    assert!(!lockfile.contains("git+https://github.com/isarmg/xcss"));
}
