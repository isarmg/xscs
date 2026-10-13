#[test]
fn embedded_xcss_revision_matches_every_locked_xcss_package() {
    let revision = env!("XCSS_REVISION");
    let lockfile = include_str!("../../../Cargo.lock");
    let sources = lockfile
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("source = \"git+https://github.com/isarmg/xcss.git?rev="))
        .collect::<Vec<_>>();
    if sources.is_empty() {
        #[cfg(debug_assertions)]
        {
            assert_eq!(revision, "unbound");
            return;
        }
        #[cfg(not(debug_assertions))]
        panic!("release builds cannot use an unbound foundation");
    }
    assert_eq!(revision.len(), 40);
    assert!(!sources.is_empty());
    assert!(
        sources
            .iter()
            .all(|source| { source.contains(&format!("?rev={revision}#{revision}\"")) })
    );
}
