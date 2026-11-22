//! Architectural layering verification for the PaperCraft workspace (`cargo xtask layers`).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::process::Command;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    pub workspace: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    Unregistered {
        krate: String,
    },
    StandaloneHasWorkspaceDep {
        from: String,
        to: String,
        kind: DepKind,
    },
    Upward {
        from: String,
        from_layer: u8,
        to: String,
        to_layer: u8,
        kind: DepKind,
    },
    SameLayer {
        from: String,
        to: String,
        layer: u8,
        kind: DepKind,
    },
    UiBelowL6 {
        from: String,
        layer: u8,
        dep: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(
                    f,
                    "unregistered workspace crate `{krate}`: register it in xtask/src/layers.rs"
                )
            }
            Violation::StandaloneHasWorkspaceDep { from, to, kind } => {
                write!(
                    f,
                    "standalone crate `{from}` must not depend on workspace crate `{to}` ({kind:?})"
                )
            }
            Violation::Upward {
                from,
                from_layer,
                to,
                to_layer,
                kind,
            } => {
                write!(
                    f,
                    "layer violation ({kind:?}): `{from}` (L{from_layer}) depends upward on `{to}` (L{to_layer})"
                )
            }
            Violation::SameLayer {
                from,
                to,
                layer,
                kind,
            } => {
                write!(
                    f,
                    "same-layer dependency ({kind:?}): `{from}` (L{layer}) depends on `{to}` (L{layer})"
                )
            }
            Violation::UiBelowL6 { from, layer, dep } => {
                write!(
                    f,
                    "UI dependency below L6: `{from}` (L{layer}) depends on `{dep}`"
                )
            }
        }
    }
}

const EXEMPT: &[&str] = &[
    "papercraft",
    "papercraft-server",
    "papercraft-server-cli",
    "papercraft-web",
    "papercraft-cli",
    "xtask",
];
const STANDALONE: &[&str] = &["papercraft-tablet"];
const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "egui_kittest", "rfd"];

fn layer_of(name: &str) -> Option<u8> {
    match name {
        "papercraft-geom" | "papercraft-color" | "papercraft-raster" | "papercraft-tablet" => {
            Some(0)
        }
        "papercraft-doc" => Some(1),
        "papercraft-ops" | "papercraft-paint" | "papercraft-sync-protocol" => Some(2),
        "papercraft-compose" | "papercraft-format" => Some(3),
        "papercraft-sync" => Some(4),
        "papercraft-engine" => Some(5),
        "papercraft-ui-egui" => Some(6),
        _ => None,
    }
}

fn l0_rank(name: &str) -> Option<u8> {
    match name {
        "papercraft-geom" => Some(0),
        "papercraft-color" => Some(1),
        "papercraft-raster" => Some(2),
        _ => None,
    }
}

pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let known: HashSet<&str> = crates.iter().map(|c| c.name.as_str()).collect();
    let mut layers: HashMap<&str, u8> = HashMap::new();

    for c in crates {
        if EXEMPT.contains(&c.name.as_str()) {
            continue;
        }
        if let Some(layer) = layer_of(&c.name) {
            layers.insert(c.name.as_str(), layer);
        } else {
            violations.push(Violation::Unregistered {
                krate: c.name.clone(),
            });
        }
    }

    for c in crates {
        if EXEMPT.contains(&c.name.as_str()) {
            continue;
        }
        let Some(&from_layer) = layers.get(c.name.as_str()) else {
            continue;
        };
        let is_standalone = STANDALONE.contains(&c.name.as_str());

        for dep in &c.deps {
            if dep.name == c.name {
                continue;
            }
            if from_layer < 6 && UI_CRATES.contains(&dep.name.as_str()) {
                violations.push(Violation::UiBelowL6 {
                    from: c.name.clone(),
                    layer: from_layer,
                    dep: dep.name.clone(),
                });
            }
            let is_ws = dep.workspace || known.contains(dep.name.as_str());
            if !is_ws {
                continue;
            }
            if is_standalone {
                violations.push(Violation::StandaloneHasWorkspaceDep {
                    from: c.name.clone(),
                    to: dep.name.clone(),
                    kind: dep.kind,
                });
                continue;
            }
            let Some(to_layer) = layers
                .get(dep.name.as_str())
                .copied()
                .or_else(|| layer_of(&dep.name))
            else {
                continue;
            };
            if to_layer > from_layer {
                violations.push(Violation::Upward {
                    from: c.name.clone(),
                    from_layer,
                    to: dep.name.clone(),
                    to_layer,
                    kind: dep.kind,
                });
            } else if to_layer == from_layer {
                let allowed_chain = from_layer == 0
                    && matches!((l0_rank(&c.name), l0_rank(&dep.name)), (Some(a), Some(b)) if a > b);
                if !allowed_chain {
                    violations.push(Violation::SameLayer {
                        from: c.name.clone(),
                        to: dep.name.clone(),
                        layer: from_layer,
                        kind: dep.kind,
                    });
                }
            }
        }
    }

    violations
}

pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let packages = meta
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "cargo metadata missing `packages` array".to_string())?;
    let members: HashSet<String> = meta
        .get("workspace_members")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut out = Vec::new();
    for pkg in packages {
        let pkg_id = pkg.get("id").and_then(Value::as_str).unwrap_or_default();
        if !members.is_empty() && !members.contains(pkg_id) {
            continue;
        }
        let name = pkg
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "package missing `name`".to_string())?
            .to_string();
        let deps_val = pkg
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("package `{name}` missing `dependencies`"))?;
        let mut deps = Vec::new();
        for d in deps_val {
            let dep_name = d
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("dependency of `{name}` missing `name`"))?
                .to_string();
            let kind = match d.get("kind").and_then(Value::as_str) {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = d.get("path").is_some();
            deps.push(Dep {
                name: dep_name,
                kind,
                workspace,
            });
        }
        out.push(Crate { name, deps });
    }
    Ok(out)
}

pub fn run() -> Result<usize, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("failed to run `cargo metadata`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("`cargo metadata` failed: {stderr}"));
    }
    let meta: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse `cargo metadata` JSON: {e}"))?;
    let crates = from_metadata(&meta)?;
    let library_count = crates
        .iter()
        .filter(|c| !EXEMPT.contains(&c.name.as_str()))
        .count();
    let violations = check(&crates);
    if violations.is_empty() {
        Ok(library_count)
    } else {
        let msg = violations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        Err(msg)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate {
            name: name.to_string(),
            deps: deps
                .iter()
                .map(|&(d, kind, workspace)| Dep {
                    name: d.to_string(),
                    kind,
                    workspace,
                })
                .collect(),
        }
    }

    #[test]
    fn valid_workspace_passes() {
        let crates = vec![
            c("papercraft-geom", &[]),
            c(
                "papercraft-color",
                &[("papercraft-geom", DepKind::Normal, true)],
            ),
            c(
                "papercraft-raster",
                &[
                    ("papercraft-geom", DepKind::Normal, true),
                    ("papercraft-color", DepKind::Normal, true),
                ],
            ),
            c("papercraft-tablet", &[("log", DepKind::Normal, false)]),
            c(
                "papercraft-doc",
                &[
                    ("papercraft-geom", DepKind::Normal, true),
                    ("papercraft-color", DepKind::Normal, true),
                ],
            ),
            c(
                "papercraft-ops",
                &[("papercraft-doc", DepKind::Normal, true)],
            ),
            c(
                "papercraft-paint",
                &[
                    ("papercraft-geom", DepKind::Normal, true),
                    ("papercraft-color", DepKind::Normal, true),
                    ("papercraft-raster", DepKind::Normal, true),
                    ("papercraft-doc", DepKind::Normal, true),
                ],
            ),
            c(
                "papercraft-compose",
                &[
                    ("papercraft-geom", DepKind::Normal, true),
                    ("papercraft-color", DepKind::Normal, true),
                    ("papercraft-raster", DepKind::Normal, true),
                    ("papercraft-doc", DepKind::Normal, true),
                    ("papercraft-paint", DepKind::Normal, true),
                ],
            ),
            c(
                "papercraft-format",
                &[("papercraft-doc", DepKind::Normal, true)],
            ),
            c(
                "papercraft-engine",
                &[
                    ("papercraft-doc", DepKind::Normal, true),
                    ("papercraft-ops", DepKind::Normal, true),
                    ("papercraft-paint", DepKind::Normal, true),
                ],
            ),
            c(
                "papercraft-ui-egui",
                &[
                    ("papercraft-engine", DepKind::Normal, true),
                    ("egui", DepKind::Normal, false),
                ],
            ),
        ];
        assert!(check(&crates).is_empty());
    }

    #[test]
    fn upward_and_same_layer_flagged() {
        let v = check(&[c(
            "papercraft-geom",
            &[("papercraft-doc", DepKind::Normal, true)],
        )]);
        assert!(matches!(
            v[..],
            [Violation::Upward {
                from_layer: 0,
                to_layer: 1,
                ..
            }]
        ));

        let v = check(&[c(
            "papercraft-ops",
            &[("papercraft-paint", DepKind::Normal, true)],
        )]);
        assert!(matches!(v[..], [Violation::SameLayer { layer: 2, .. }]));

        let v = check(&[c(
            "papercraft-geom",
            &[("papercraft-raster", DepKind::Normal, true)],
        )]);
        assert!(matches!(v[..], [Violation::SameLayer { layer: 0, .. }]));
    }

    #[test]
    fn standalone_and_ui_below_l6_flagged() {
        let v = check(&[c(
            "papercraft-tablet",
            &[("papercraft-geom", DepKind::Normal, true)],
        )]);
        assert!(matches!(
            v[..],
            [Violation::StandaloneHasWorkspaceDep { .. }]
        ));

        let v = check(&[c("papercraft-engine", &[("egui", DepKind::Normal, false)])]);
        assert!(matches!(v[..], [Violation::UiBelowL6 { layer: 5, .. }]));
    }

    #[test]
    fn unregistered_crate_flagged() {
        let v = check(&[c("papercraft-mystery", &[])]);
        assert!(
            matches!(&v[..], [Violation::Unregistered { krate }] if krate == "papercraft-mystery")
        );
    }

    #[test]
    fn metadata_parsing_filters_non_workspace_members() {
        let meta = serde_json::json!({
            "workspace_members": ["papercraft-doc 0.1.0"],
            "packages": [
                {
                    "id": "papercraft-doc 0.1.0",
                    "name": "papercraft-doc",
                    "dependencies": [
                        {"name": "papercraft-geom", "kind": null, "path": "/crates/geom"},
                        {"name": "serde", "kind": null}
                    ]
                },
                {
                    "id": "serde 1.0.0",
                    "name": "serde",
                    "dependencies": []
                }
            ]
        });
        let crates = from_metadata(&meta).unwrap();
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].name, "papercraft-doc");
        assert!(crates[0].deps[0].workspace);
        assert!(!crates[0].deps[1].workspace);
    }
}
