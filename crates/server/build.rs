#[path = "src/config_definition.rs"]
mod config_definition;
#[path = "src/database_definition.rs"]
mod database_definition;
#[path = "src/installation_definition.rs"]
mod installation_definition;
#[path = "src/persistent_crypto_definition.rs"]
mod persistent_crypto_definition;
#[path = "src/secret_definition.rs"]
mod secret_definition;
#[path = "../../scripts/server-target-gate.rs"]
mod target_gate;
use std::{env, fs};

fn local_foundation_revision(lockfile: &str, source_revision: &str) -> String {
    let packages = lockfile
        .split("[[package]]")
        .filter(|package| {
            package
                .lines()
                .any(|line| line.trim() == r#"name = "xscs-foundation""#)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        packages.len(),
        1,
        "one product-owned foundation is required"
    );
    assert!(
        !packages[0]
            .lines()
            .any(|line| line.trim().starts_with("source =")),
        "the product foundation must resolve from its workspace path"
    );
    if source_revision == "unbound" {
        assert_eq!(
            env::var("PROFILE").as_deref(),
            Ok("debug"),
            "release builds require a source-bound product foundation"
        );
        assert!(
            env::var_os("XSSI_BUILD_METADATA_DIR").is_none(),
            "release metadata requires a source-bound product foundation"
        );
    }
    source_revision.to_owned()
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
    let xcss_revision = local_foundation_revision(&lockfile, &source_revision);
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
    emit_installation_metadata();
}

fn emit_installation_metadata() {
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
    let hash = database_definition::SCHEMA_SHA256;
    let schema = xcss::contracts::SchemaIdentity::new(
        database_definition::APPLICATION,
        database_definition::SCHEMA_APPLICATION_VERSION,
        u64::try_from(database_definition::SCHEMA_REVISION)
            .expect("schema revision is non-negative"),
        hash,
    )
    .expect("schema identity");
    let contract = StateContract {
        contract_version: xcss::contracts::STATE_CONTRACT_VERSION,
        application: database_definition::APPLICATION.into(),
        application_version: std::env::var("CARGO_PKG_VERSION").unwrap(),
        source_revision: revision.clone(),
        schema: Some(StateSchemaIdentity {
            revision: u64::try_from(database_definition::SCHEMA_REVISION)
                .expect("schema revision is non-negative"),
            sha256: hash.into(),
        }),
        maintenance_locks: vec![".state-instance.lock".into()],
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
        product: database_definition::APPLICATION.into(),
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
    installation_definition::emit(
        &destination,
        &identity,
        &schema,
        include_str!("../../schema/generated/current_schema.sql"),
    );
}
