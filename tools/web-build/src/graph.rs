//! Verify integrated sources and Cargo's conservative resolution graph.
//! Metadata may include features from conditional edges that the current host
//! compilation does not activate. This policy guard is not a compiled-unit
//! inventory; host proc-macros are never counted as browser WASM code.
use crate::{Options, inventory};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

pub fn resolve(options: &Options) -> Result<Value> {
    let frontend = options.root.join("web/leptos");
    let mut command = Command::new("cargo");
    command
        .arg(format!("+{}", crate::RUST))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            crate::TARGET,
            "--manifest-path",
        ])
        .arg(frontend.join("Cargo.toml"))
        .current_dir(&options.root);
    if options.offline {
        command.arg("--offline");
    }
    let output = command
        .output()
        .context("Inspect selected CSR dependency graph")?;
    ensure!(
        output.status.success(),
        "Cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        output.stdout.len() <= 32 * 1024 * 1024,
        "Cargo metadata exceeds byte bound"
    );
    validate(&serde_json::from_slice(&output.stdout)?, &frontend)
}

pub fn validate(metadata: &Value, frontend: &Path) -> Result<Value> {
    let packages = metadata["packages"]
        .as_array()
        .context("Cargo metadata packages")?;
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .context("Cargo metadata resolve nodes")?;
    let by_id = packages
        .iter()
        .map(|p| Ok((p["id"].as_str().context("Cargo package id")?, p)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let root = metadata["resolve"]["root"]
        .as_str()
        .context("Standalone browser crate must have a Cargo resolve root")?;
    let node_ids = nodes
        .iter()
        .map(|p| Ok((p["id"].as_str().context("Cargo node id")?, p)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut reachable = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let node = node_ids
            .get(id)
            .context("Missing resolved dependency node")?;
        for dependency in node["dependencies"]
            .as_array()
            .context("Dependency node edges")?
        {
            pending.push(dependency.as_str().context("Dependency id")?);
        }
    }
    let integrated = if by_id[root]["name"] == "xcss-web" {
        frontend.to_owned()
    } else {
        frontend.join("vendor/xcss-web")
    };
    let mut selected = Vec::new();
    for (name, directory) in [
        ("leptos", "leptos-csr"),
        ("leptos_macro", "leptos-macro-csr"),
    ] {
        let candidates = reachable
            .iter()
            .filter_map(|id| {
                by_id
                    .get(id)
                    .filter(|p| p["name"] == name)
                    .map(|p| (*id, *p))
            })
            .collect::<Vec<_>>();
        ensure!(
            candidates.len() == 1,
            "CSR requires exactly one integrated {name}; found {}",
            candidates.len()
        );
        let (id, package) = candidates[0];
        let expected = integrated.join("vendor").join(directory).join("Cargo.toml");
        let actual = Path::new(
            package["manifest_path"]
                .as_str()
                .context("Selected package manifest")?,
        );
        ensure!(
            package["source"].is_null() && actual.canonicalize()? == expected.canonicalize()?,
            "{name} must resolve to the xcss integrated CSR source"
        );
        let features = node_ids[id]["features"]
            .as_array()
            .context("Resolved package features")?
            .iter()
            .map(|v| v.as_str().context("Feature name"))
            .collect::<Result<BTreeSet<_>>>()?;
        let allowed = if name == "leptos_macro" {
            BTreeSet::from(["csr", "__internal_erase_components"])
        } else {
            BTreeSet::from(["csr"])
        };
        ensure!(
            features.contains("csr") && features.is_subset(&allowed),
            "{name} only permits csr and the host macro's internal erase-components flag; selected {features:?}"
        );
        let host_macro = package["targets"]
            .as_array()
            .context("Package targets")?
            .iter()
            .any(|target| {
                target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "proc-macro"))
            });
        ensure!(
            host_macro == (name == "leptos_macro"),
            "Integrated runtime/proc-macro target roles changed"
        );
        selected.push(json!({"name":name,"version":package["version"],"path":inventory::relative(expected.strip_prefix(frontend)?)?,"features":features,"role":if host_macro {"host-proc-macro"} else {"browser"}}));
    }
    for id in reachable {
        let package = by_id[id];
        let name = package["name"].as_str().context("Package name")?;
        ensure!(
            !matches!(
                name,
                "server_fn"
                    | "server_fn_macro"
                    | "leptos_server"
                    | "hydration_context"
                    | "leptos_config"
            ),
            "Server/hydration package unexpectedly selected in CSR graph: {name}"
        );
        if matches!(name, "reactive_graph" | "tachys") {
            let features = node_ids[id]["features"]
                .as_array()
                .context("Dependency features")?;
            ensure!(
                !features.iter().any(|feature| matches!(
                    feature.as_str(),
                    Some("ssr" | "hydrate" | "hydration" | "islands")
                )),
                "{name} selected a server/hydration feature"
            );
        }
    }
    Ok(
        json!({"target":crate::TARGET,"mode":"csr","feature_scope":"cargo-resolution-union","integrated":selected}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(tempfile::TempDir, Value)> {
        let temp = tempfile::tempdir()?;
        let frontend = temp.path();
        for name in ["leptos-csr", "leptos-macro-csr"] {
            std::fs::create_dir_all(frontend.join("vendor/xcss-web/vendor").join(name))?;
            std::fs::write(
                frontend
                    .join("vendor/xcss-web/vendor")
                    .join(name)
                    .join("Cargo.toml"),
                "test",
            )?;
        }
        let core = frontend.join("vendor/xcss-web/vendor/leptos-csr/Cargo.toml");
        let macros = frontend.join("vendor/xcss-web/vendor/leptos-macro-csr/Cargo.toml");
        let value = json!({"packages":[
            {"id":"root","name":"product-web","version":"1","source":null,"manifest_path":frontend.join("Cargo.toml"),"targets":[{"kind":["cdylib"]}]},
            {"id":"core","name":"leptos","version":"0.8.19","source":null,"manifest_path":core,"targets":[{"kind":["lib"]}]},
            {"id":"macro","name":"leptos_macro","version":"0.8.17","source":null,"manifest_path":macros,"targets":[{"kind":["proc-macro"]}]},
            {"id":"graph","name":"reactive_graph","version":"0.2","source":"registry","manifest_path":"registry","targets":[{"kind":["lib"]}]}
        ],"resolve":{"root":"root","nodes":[
            {"id":"root","dependencies":["core"],"features":["default"]},
            {"id":"core","dependencies":["macro","graph"],"features":["csr"]},
            {"id":"macro","dependencies":[],"features":["csr"]},
            {"id":"graph","dependencies":[],"features":["default","effects"]}
        ]}});
        Ok((temp, value))
    }
    #[test]
    fn csr_graph_distinguishes_host_macros_and_unrelated_default_features() -> Result<()> {
        let (temp, value) = fixture()?;
        let receipt = validate(&value, temp.path())?;
        assert_eq!(receipt["integrated"][1]["role"], "host-proc-macro");
        Ok(())
    }
    #[test]
    fn csr_graph_rejects_full_and_hydration_and_registry_runtime() -> Result<()> {
        let (temp, value) = fixture()?;
        let mut full = value.clone();
        full["resolve"]["nodes"][1]["features"] = json!(["csr", "full"]);
        assert!(validate(&full, temp.path()).is_err());
        let mut hydration = value.clone();
        hydration["resolve"]["nodes"][3]["features"] = json!(["default", "hydration"]);
        assert!(validate(&hydration, temp.path()).is_err());
        let mut registry = value;
        registry["packages"][1]["source"] = json!("registry");
        assert!(validate(&registry, temp.path()).is_err());
        Ok(())
    }
}
