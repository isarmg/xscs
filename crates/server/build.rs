#[path = "../../scripts/server-target-gate.rs"]
mod target_gate;
use std::{collections::BTreeSet, env, fs};

const XCSS_SOURCE: &str = "git+https://github.com/isarmg/xcss.git?rev=";

fn locked_xcss_revision(lockfile: &str) -> String {
    let mut revisions = BTreeSet::new();
    for line in lockfile.lines().map(str::trim) {
        let Some(source) = line
            .strip_prefix("source = \"")
            .and_then(|value| value.strip_suffix('"'))
        else {
            continue;
        };
        let Some(rest) = source.strip_prefix(XCSS_SOURCE) else {
            continue;
        };
        let (requested, locked) = rest
            .split_once('#')
            .expect("xcss Cargo.lock source must contain a locked commit");
        assert!(
            requested == locked
                && locked.len() == 40
                && locked
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
            "xcss Cargo.lock source must bind one exact lowercase 40-hex revision"
        );
        revisions.insert(locked.to_owned());
    }
    if revisions.is_empty() {
        assert_eq!(
            env::var("PROFILE").as_deref(),
            Ok("debug"),
            "release builds require a source-bound xcss dependency"
        );
        assert!(
            env::var_os("XSSI_BUILD_METADATA_DIR").is_none(),
            "release metadata cannot describe an unbound foundation"
        );
        return "unbound".into();
    }
    assert_eq!(
        revisions.len(),
        1,
        "the xcss package must use one locked revision"
    );
    revisions.into_iter().next().expect("checked non-empty")
}

fn main() {
    println!("cargo:rerun-if-env-changed=XSSI_BUILD_METADATA_DIR");
    target_gate::main();
    let product_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("server crate belongs to the product workspace");
    let web_root = env::var_os("XCSS_WEB_DIST")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| product_root.join("web/dist"));
    xcss::web_assets::build::generate(&web_root).expect("generate embedded Web assets");
    println!("cargo:rerun-if-env-changed=XCSS_WEB_DIST");
    let source_revision = env::var("XSCS_SOURCE_REVISION").unwrap_or_else(|_| "unbound".to_owned());
    assert!(
        source_revision == "unbound"
            || (source_revision.len() == 40
                && source_revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))),
        "XSCS_SOURCE_REVISION must be a full lowercase 40-hex Git commit"
    );
    println!("cargo:rustc-env=XSCS_SOURCE_REVISION={source_revision}");
    let lockfile = fs::read_to_string(product_root.join("Cargo.lock")).expect("read Cargo.lock");
    let xcss_revision = locked_xcss_revision(&lockfile);
    println!("cargo:rustc-env=XCSS_REVISION={xcss_revision}");
    println!("cargo:rerun-if-env-changed=XSCS_SOURCE_REVISION");
    println!(
        "cargo:rerun-if-changed={}",
        product_root.join("Cargo.lock").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        product_root.join("release.json").display()
    );
    emit_offline_metadata();
}

fn emit_offline_metadata() {
    use sha2::{Digest, Sha256};
    use xcss::contracts::{StateContract, StateResource, StateResourceKind, StateSchemaIdentity};
    let Some(destination) = std::env::var_os("XSSI_BUILD_METADATA_DIR") else {
        return;
    };
    let destination = std::path::PathBuf::from(destination);
    assert!(
        destination.is_absolute() && destination.is_dir(),
        "explicit metadata destination must exist and be absolute"
    );
    let revision = std::env::var("XSCS_SOURCE_REVISION")
        .expect("release metadata requires a source-bound build");
    let hash = "b3fdff2217ea2ba4a384e3ade29cbf87a949d63392bdff3895916ff2377bd1ba";
    let schema =
        xcss::contracts::SchemaIdentity::new("xscs", "1.0.0", 1, hash).expect("schema identity");
    let contract = StateContract {
        contract_version: xcss::contracts::STATE_CONTRACT_VERSION,
        application: "xscs".into(),
        application_version: std::env::var("CARGO_PKG_VERSION").unwrap(),
        source_revision: revision.clone(),
        schema: Some(StateSchemaIdentity {
            revision: 1,
            sha256: hash.into(),
        }),
        maintenance_locks: vec![
            ".state-instance.lock".into(),
            ".state-maintenance.lock".into(),
        ],
        resources: vec![
            StateResource {
                name: "state".into(),
                kind: StateResourceKind::DataTree,
                required: true,
            },
            StateResource {
                name: "config".into(),
                kind: StateResourceKind::Configuration,
                required: true,
            },
        ],
        external_requirements: vec![],
        companion_contracts: vec![],
    };
    contract.validate().expect("current state contract");
    let bytes = serde_json::to_vec(&contract).unwrap();
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let identity = xcss::contracts::ReleaseIdentity {
        product: "xscs".into(),
        version: std::env::var("CARGO_PKG_VERSION").unwrap(),
        source_revision: revision,
        target: xcss::server_target::SERVER_TARGET_TRIPLE.into(),
        state_contract_sha256: digest,
    };
    identity.validate().expect("release identity");
    std::fs::write(
        destination.join("xscs-identity.json"),
        serde_json::to_vec(&identity).unwrap(),
    )
    .unwrap();
    std::fs::write(
        destination.join("xscs-schema.json"),
        serde_json::to_vec(&schema).unwrap(),
    )
    .unwrap();
    std::fs::write(destination.join("xscs-state-contract.json"), bytes).unwrap();
    let generated = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
        .join("xcss-web-assets.rs.manifest.json");
    std::fs::copy(generated, destination.join("xscs-web-assets.json"))
        .expect("copy exact embedded Web manifest");
}
