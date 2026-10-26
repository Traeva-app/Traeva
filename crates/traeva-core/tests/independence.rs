//! The core does not depend on Tauri, directly or through another crate
//! (architecture.md; organize-spec.md O36). Read from the workspace's
//! Cargo.lock, which `cargo` keeps in step with the manifests.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

/// Package name → the names of its dependencies, from Cargo.lock.
fn dependencies_in_lock(lock: &str) -> HashMap<String, Vec<String>> {
    let mut packages = HashMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let name = block
            .lines()
            .find_map(|line| line.strip_prefix("name = "))
            .unwrap()
            .trim_matches('"')
            .to_string();
        let mut deps = Vec::new();
        if let Some(start) = block.find("dependencies = [") {
            let list = &block[start..block[start..].find(']').unwrap() + start];
            for item in list.split('"').skip(1).step_by(2) {
                // "name" or "name version" or "name version (source)".
                deps.push(item.split(' ').next().unwrap().to_string());
            }
        }
        packages.entry(name).or_insert_with(Vec::new).extend(deps);
    }
    packages
}

#[test]
fn the_core_does_not_depend_on_tauri() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock = fs::read_to_string(workspace.join("Cargo.lock")).unwrap();
    let packages = dependencies_in_lock(&lock);
    assert!(packages.contains_key("tauri"), "the app itself uses Tauri");

    let mut seen = BTreeSet::new();
    let mut next = vec!["traeva-core".to_string()];
    while let Some(name) = next.pop() {
        if seen.insert(name.clone()) {
            next.extend(packages.get(&name).cloned().unwrap_or_default());
        }
    }
    assert!(seen.len() > 10, "the walk found the core's dependencies");
    let tauri: Vec<_> = seen.iter().filter(|name| name.contains("tauri")).collect();
    assert!(tauri.is_empty(), "the core reaches {tauri:?}");

    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("tauri ="),
        "no Tauri entry in the manifest"
    );
}
