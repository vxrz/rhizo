//! Which providers OpenTofu may install and run (docs/design/iac-import.md,
//! "Security: which code runs"). Every provider is mocked, but `tofu init`
//! still downloads each one the code requires and OpenTofu starts it for its
//! schema: a provider is a program from its publisher that runs on this
//! machine. So only the providers below run, unless a person allows more
//! for one run.

use std::path::Path;

use hcl_edit::expr::{Expression, ObjectKey};
use hcl_edit::structure::Body;
use serde_json::Value;

use crate::Error;
use crate::hcl::LOCK;

const HOST: &str = "registry.opentofu.org";

/// The providers installed without asking. Named one by one: the registry's
/// `hashicorp` namespace also serves other publishers' programs under legacy
/// names (`hashicorp/github` is `integrations/github`, `hashicorp/cloudflare`
/// is Cloudflare's), so `hashicorp/*` would let those in.
pub const ALLOWED: &[&str] = &[
    // The clouds rhizo compiles to, and the beta Google provider the
    // terraform-google-modules require.
    "registry.opentofu.org/hashicorp/aws",
    "registry.opentofu.org/hashicorp/azurerm",
    "registry.opentofu.org/hashicorp/google",
    "registry.opentofu.org/hashicorp/google-beta",
    // HashiCorp's utility providers, which the modules rhizo emits require.
    "registry.opentofu.org/hashicorp/random",
    "registry.opentofu.org/hashicorp/null",
    "registry.opentofu.org/hashicorp/local",
    "registry.opentofu.org/hashicorp/external",
    "registry.opentofu.org/hashicorp/time",
    "registry.opentofu.org/hashicorp/archive",
    "registry.opentofu.org/hashicorp/tls",
    "registry.opentofu.org/hashicorp/http",
    // Microsoft's, required by the Azure Verified Modules rhizo emits.
    "registry.opentofu.org/azure/azapi",
    "registry.opentofu.org/azure/modtm",
];

/// A provider the person allows: `NAMESPACE/NAME` or `HOST/NAMESPACE/NAME`,
/// as the full lowercase address. No wildcards: each one is a decision.
pub fn parse_allowed(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.trim().split('/').collect();
    let ok = |p: &&str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    match parts[..] {
        [_, _] | [_, _, _] if parts.iter().all(ok) => Ok(address(s.trim())),
        _ => {
            Err(format!("`{s}` is not a provider: write NAMESPACE/NAME (`integrations/github`) or HOST/NAMESPACE/NAME"))
        }
    }
}

/// A provider source as written in code, as OpenTofu reads it: `aws`,
/// `hashicorp/aws` and `registry.opentofu.org/hashicorp/aws` are the same.
fn address(source: &str) -> String {
    let s = source.to_ascii_lowercase();
    match s.split('/').count() {
        1 => format!("{HOST}/hashicorp/{s}"),
        2 => format!("{HOST}/{s}"),
        _ => s,
    }
}

/// How the refusal names a provider: as `--allow-provider` takes it.
pub fn short(address: &str) -> &str {
    address.strip_prefix("registry.opentofu.org/").unwrap_or(address)
}

/// The CLI configuration every `tofu` run gets instead of the person's:
/// direct installation from the registry, of the allowed providers only.
/// No mirrors and no `dev_overrides`, which install or run other programs.
pub fn cli_config(allowed: &[String]) -> String {
    let list: Vec<String> = allowed.iter().map(|a| format!("      \"{a}\",\n")).collect();
    format!(
        "# Written by rhizo for one run; the person's own CLI configuration is not read.\nprovider_installation {{\n  direct {{\n    include = [\n{}    ]\n  }}\n}}\n",
        list.concat()
    )
}

/// Every provider the code in `dir` names, with where: its configuration
/// files, its lock file, and the modules `tofu get` installed there. A
/// locked provider already in the plugin cache is used without asking the
/// registry, so OpenTofu's own `include` does not cover the lock file.
pub fn required(dir: &Path) -> Result<Vec<(String, String)>, Error> {
    let mut found = in_module(dir, "the root module", "");
    if let Ok(text) = std::fs::read_to_string(dir.join(LOCK)) {
        let body = hcl_edit::parser::parse_body(&text).map_err(|e| Error::Io(format!("{LOCK} cannot be read: {e}")))?;
        let locked = body.blocks().filter(|b| b.ident.as_str() == "provider").filter_map(|b| b.labels.first());
        found.extend(locked.map(|l| (address(l.as_str()), format!("the lock file {LOCK}"))));
    }
    let manifest = std::fs::read_to_string(dir.join(".terraform/modules/modules.json")).unwrap_or_default();
    let manifest: Value = serde_json::from_str(&manifest).unwrap_or_default();
    for m in manifest["Modules"].as_array().into_iter().flatten() {
        let (Some(key), Some(sub)) = (m["Key"].as_str(), m["Dir"].as_str()) else { continue };
        if !key.is_empty() {
            let at = format!("module.{}", key.replace('.', ".module."));
            let from = m["Source"].as_str().map(|s| format!(", source \"{s}\"")).unwrap_or_default();
            found.extend(in_module(&dir.join(sub), &at, &from));
        }
    }
    Ok(found)
}

/// The providers a module's `terraform { required_providers }` names.
fn in_module(dir: &Path, at: &str, from: &str) -> Vec<(String, String)> {
    declared(dir).into_iter().map(|(name, _, address)| (address, format!("{at} ({name}{from})"))).collect()
}

/// The root module's local provider names with their addresses (`ext` for
/// `hashicorp/external`): `tofu test` mocks a configuration by its local
/// name, so each name needs a mock of its own.
pub fn local_names(dir: &Path) -> Vec<(String, String)> {
    declared(dir).into_iter().map(|(_, local, address)| (local, address)).collect()
}

/// Each `required_providers` entry of the module in `dir`: file, local name, address.
fn declared(dir: &Path) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (name, body) in bodies(dir) {
        for (local, expr) in required_providers(&body) {
            let address = source(&local, expr);
            out.push((name.clone(), local, address));
        }
    }
    out
}

/// The parsed `.tf` and `.tofu` files of the module in `dir`, by name.
// shortcut: reads `.tf` and `.tofu` files, not `.tf.json`; OpenTofu's `include` still refuses those at init.
fn bodies(dir: &Path) -> Vec<(String, Body)> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tf") || n.ends_with(".tofu"))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap_or_default();
            // Unparseable code fails in `tofu` with its own message.
            let body = hcl_edit::parser::parse_body(&text).ok()?;
            Some((name, body))
        })
        .collect()
}

/// `provider` blocks in the modules `tofu get` installed under `dir`.
/// `tofu test` mocks only the root module's provider configurations, so
/// one a module configures itself would run for real.
fn configured_in_modules(dir: &Path) -> Vec<String> {
    let manifest = std::fs::read_to_string(dir.join(".terraform/modules/modules.json")).unwrap_or_default();
    let manifest: Value = serde_json::from_str(&manifest).unwrap_or_default();
    let mut out = Vec::new();
    for m in manifest["Modules"].as_array().into_iter().flatten() {
        let (Some(key), Some(sub)) = (m["Key"].as_str(), m["Dir"].as_str()) else { continue };
        if key.is_empty() {
            continue;
        }
        let at = format!("module.{}", key.replace('.', ".module."));
        for (name, body) in bodies(&dir.join(sub)) {
            for b in body.blocks().filter(|b| b.ident.as_str() == "provider") {
                let label = b.labels.first().map(|l| l.as_str().to_string()).unwrap_or_default();
                out.push(format!("{at} ({name}): provider \"{label}\""));
            }
        }
        // JSON configuration is not read above, so it could hide a `provider` block.
        let json = std::fs::read_dir(dir.join(sub)).into_iter().flatten().flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n.ends_with(".tf.json") || n.ends_with(".tofu.json")).then_some(n)
        });
        out.extend(json.map(|n| format!("{at} ({n}): JSON configuration, which rhizo does not check")));
    }
    out
}

/// Module sources in the root module in `dir` and the modules installed
/// there that read outside the imported folder: an absolute path (which
/// OpenTofu links to) or a `file://` URL (which it links or clones).
fn outside_sources(dir: &Path) -> Vec<String> {
    let outside = |s: &str| {
        let b = s.as_bytes();
        s.to_ascii_lowercase().contains("file://")
            || s.starts_with(['/', '\\', '~'])
            || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
    };
    let mut out = Vec::new();
    for (name, body) in bodies(dir) {
        for b in body.blocks().filter(|b| b.ident.as_str() == "module") {
            let source = b.body.get_attribute("source").and_then(|a| a.value.as_str()).unwrap_or_default();
            if outside(source) {
                out.push(format!("the root module ({name}): source \"{source}\""));
            }
        }
    }
    let manifest = std::fs::read_to_string(dir.join(".terraform/modules/modules.json")).unwrap_or_default();
    let manifest: Value = serde_json::from_str(&manifest).unwrap_or_default();
    for m in manifest["Modules"].as_array().into_iter().flatten() {
        let (Some(key), Some(source)) = (m["Key"].as_str(), m["Source"].as_str()) else { continue };
        if outside(source) {
            out.push(format!("module.{} (source \"{source}\")", key.replace('.', ".module.")));
        }
    }
    out
}

fn required_providers(body: &Body) -> Vec<(String, &Expression)> {
    body.blocks()
        .filter(|b| b.ident.as_str() == "terraform")
        .flat_map(|b| b.body.blocks().filter(|r| r.ident.as_str() == "required_providers"))
        .flat_map(|r| r.body.attributes().map(|a| (a.key.as_str().to_string(), &a.value)))
        .collect()
}

/// `aws = { source = "hashicorp/aws" }`; without `source` (or the legacy
/// version string) the local name is a `hashicorp` provider.
fn source(local: &str, expr: &Expression) -> String {
    let given = match expr {
        Expression::Object(o) => o.iter().find_map(|(k, v)| {
            let key = match k {
                ObjectKey::Ident(i) => i.as_str(),
                ObjectKey::Expression(Expression::Variable(v)) => v.as_str(),
                ObjectKey::Expression(Expression::String(s)) => s.as_str(),
                _ => return None,
            };
            match v.expr() {
                Expression::String(s) if key == "source" => Some(s.value().clone()),
                _ => None,
            }
        }),
        _ => None,
    };
    address(&given.unwrap_or_else(|| local.to_string()))
}

/// Refuses every provider the code in `dir` requires that `allowed` does
/// not name, before `tofu` installs it.
pub fn check(dir: &Path, allowed: &[String]) -> Result<(), Error> {
    let mut refused: Vec<(String, String)> =
        required(dir)?.into_iter().filter(|(p, _)| !allowed.iter().any(|a| a == p)).collect();
    refused.dedup();
    if !refused.is_empty() {
        return Err(Error::NotAllowed(refused));
    }
    let outside = outside_sources(dir);
    if !outside.is_empty() {
        return Err(Error::Io(format!(
            "a module's source is outside the imported folder, which rhizo does not read; nothing was run:\n  - {}",
            outside.join("\n  - ")
        )));
    }
    let configured = configured_in_modules(dir);
    if configured.is_empty() {
        Ok(())
    } else {
        Err(Error::Io(format!(
            "a module configures its own provider, which the offline plan cannot mock; nothing was run:\n  - {}",
            configured.join("\n  - ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rhizo-providers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (path, text) in files {
            let p = d.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        d
    }

    #[test]
    fn allowed_names_are_addresses() {
        assert_eq!(parse_allowed("integrations/github").unwrap(), "registry.opentofu.org/integrations/github");
        assert_eq!(parse_allowed("Example.com/Acme/Thing").unwrap(), "example.com/acme/thing");
        for bad in ["github", "a/*", "a/b/c/d", "a//b", "a/b\"", "a /b", "a/b\nc"] {
            assert!(parse_allowed(bad).is_err(), "{bad:?}");
        }
        assert!(ALLOWED.iter().all(|a| parse_allowed(a).as_deref() == Ok(*a)));
    }

    #[test]
    fn the_cli_config_installs_only_the_allowed_providers() {
        let config = cli_config(&["registry.opentofu.org/hashicorp/aws".into(), "registry.opentofu.org/a/b".into()]);
        assert_eq!(
            config,
            "# Written by rhizo for one run; the person's own CLI configuration is not read.\nprovider_installation {\n  direct {\n    include = [\n      \"registry.opentofu.org/hashicorp/aws\",\n      \"registry.opentofu.org/a/b\",\n    ]\n  }\n}\n"
        );
        assert!(hcl_edit::parser::parse_body(&config).is_ok());
    }

    #[test]
    fn renamed_providers_and_module_configurations_are_found() {
        let d = dir(
            "local",
            &[
                (
                    "main.tf",
                    "terraform {\n  required_providers {\n    ext = { source = \"hashicorp/external\" }\n  }\n}\n",
                ),
                (
                    ".terraform/modules/modules.json",
                    r#"{"Modules":[{"Key":"","Dir":"."},{"Key":"m","Dir":"m"},{"Key":"j","Dir":"j"}]}"#,
                ),
                ("m/main.tf", "provider \"external\" {}\n"),
                ("j/main.tf.json", "{}"),
            ],
        );
        assert_eq!(local_names(&d), [("ext".to_string(), "registry.opentofu.org/hashicorp/external".to_string())]);
        let allowed: Vec<String> = ALLOWED.iter().map(|s| s.to_string()).collect();
        let Err(Error::Io(e)) = check(&d, &allowed) else { panic!("a module's own provider is not refused") };
        assert!(
            e.contains("module.m (main.tf): provider \"external\"") && e.contains("module.j (main.tf.json)"),
            "{e}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// OpenTofu links an absolute module path into `.terraform/modules`
    /// (and clones a `file://` one): code from outside the folder.
    #[test]
    fn module_sources_outside_the_folder_are_refused() {
        let allowed: Vec<String> = ALLOWED.iter().map(|s| s.to_string()).collect();
        for source in ["/etc", "file:///etc", "git::file:///tmp/repo", "C:\\\\x", "~/m"] {
            let main = format!("module \"m\" {{\n  source = {:?}\n}}\n", source);
            let d = dir("outside", &[("main.tf", &main)]);
            let Err(Error::Io(e)) = check(&d, &allowed) else { panic!("{source} is not refused") };
            assert!(e.contains("outside the imported folder") && e.contains("the root module (main.tf)"), "{e}");
        }
        let nested = r#"{"Modules":[{"Key":"","Source":"","Dir":"."},{"Key":"a","Source":"registry.opentofu.org/x/a/aws","Dir":".terraform/modules/a"},{"Key":"a.b","Source":"file:///Users/me","Dir":".terraform/modules/a.b"}]}"#;
        let d = dir(
            "outside",
            &[("main.tf", "module \"a\" {\n  source = \"x/a/aws\"\n}\n"), (".terraform/modules/modules.json", nested)],
        );
        let Err(Error::Io(e)) = check(&d, &allowed) else { panic!("a nested module outside is not refused") };
        assert!(e.contains("module.a.module.b (source \"file:///Users/me\")") && !e.contains("module.a ("), "{e}");
        let d = dir(
            "outside",
            &[(
                "main.tf",
                "module \"a\" {\n  source = \"./a\"\n}\nmodule \"b\" {\n  source = \"git::https://example.com/b.git?ref=v1\"\n}\n",
            )],
        );
        assert!(check(&d, &allowed).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn requirements_come_from_root_lock_and_modules() {
        let d = dir(
            "req",
            &[
                (
                    "main.tf",
                    "terraform {\n  required_providers {\n    aws = { source = \"hashicorp/aws\" }\n    gh = {\n      \"source\" = \"Integrations/GitHub\"\n    }\n    random = \"~> 3.0\"\n    tls = {}\n  }\n}\nmodule \"m\" { source = \"./m\" }\n",
                ),
                (
                    "other.tofu",
                    "terraform {\n  required_providers {\n    x = { source = \"example.com/acme/x\" }\n  }\n}\n",
                ),
                ("notes.txt", "terraform { required_providers { y = { source = \"evil/y\" } } }"),
                (
                    LOCK,
                    "provider \"registry.opentofu.org/hashicorp/aws\" {\n  version = \"5.0.0\"\n}\nprovider \"registry.opentofu.org/cloudflare/cloudflare\" {\n  version = \"5.0.0\"\n}\n",
                ),
                (
                    ".terraform/modules/modules.json",
                    r#"{"Modules":[{"Key":"","Source":"","Dir":"."},{"Key":"m","Source":"./m","Dir":"m"},{"Key":"m.n","Source":"registry.opentofu.org/a/n/aws","Version":"1.0.0","Dir":".terraform/modules/m.n"}]}"#,
                ),
                (
                    "m/main.tf",
                    "terraform {\n  required_providers {\n    gh = { source = \"integrations/github\" }\n  }\n}\n",
                ),
                (
                    ".terraform/modules/m.n/versions.tf",
                    "terraform {\n  required_providers {\n    cf = { source = \"cloudflare/cloudflare\" }\n  }\n}\n",
                ),
            ],
        );
        let found = required(&d).unwrap();
        let r = |p: &str, w: &str| (format!("registry.opentofu.org/{p}"), w.to_string());
        assert_eq!(
            found,
            [
                r("hashicorp/aws", "the root module (main.tf)"),
                r("integrations/github", "the root module (main.tf)"),
                r("hashicorp/random", "the root module (main.tf)"),
                r("hashicorp/tls", "the root module (main.tf)"),
                ("example.com/acme/x".into(), "the root module (other.tofu)".into()),
                r("hashicorp/aws", "the lock file .terraform.lock.hcl"),
                r("cloudflare/cloudflare", "the lock file .terraform.lock.hcl"),
                r("integrations/github", "module.m (main.tf, source \"./m\")"),
                r("cloudflare/cloudflare", "module.m.module.n (versions.tf, source \"registry.opentofu.org/a/n/aws\")"),
            ]
        );
        let allowed: Vec<String> = ALLOWED.iter().map(|s| s.to_string()).collect();
        let Err(Error::NotAllowed(refused)) = check(&d, &allowed) else { panic!("not refused") };
        assert_eq!(refused.len(), 5);
        assert!(refused.iter().all(|(p, _)| !p.contains("hashicorp")));
        let mut more = allowed;
        more.extend(
            ["integrations/github", "cloudflare/cloudflare", "example.com/acme/x"].map(|p| parse_allowed(p).unwrap()),
        );
        assert!(check(&d, &more).is_ok());
        // A lock file that cannot be read could hide a cached provider: refused.
        std::fs::write(d.join(LOCK), "provider \"a/b\" {").unwrap();
        assert!(matches!(check(&d, &more), Err(Error::Io(e)) if e.contains(LOCK)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
