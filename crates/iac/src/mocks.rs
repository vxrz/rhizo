//! Mock profiles for the offline plan (docs/design/infra-cloud-neutral.md §1).
//! `tofu test` replaces every provider with `mock_provider`, so no provider
//! is configured, logs in or calls a cloud. Providers still validate the
//! values they get, so the defaults here make mocked values valid: both
//! sides of L1 see the same ones, so they need to be valid, not real.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::hcl::{self, Block};

/// The test file rhizo writes into each planning copy.
pub const TEST_FILE: &str = "zz_rhizo_l1.tftest.hcl";

/// azurerm resource type → an example ID of its shape (azurerm parses IDs
/// by resource kind, so a container needs a storage account's ID shape).
const AZURERM_IDS: &str = include_str!("mocks/azurerm_ids.tsv");

const UUID: &str = "00000000-0000-0000-0000-000000000000";
const ACCOUNT: &str = "123456789012";
const POLICY: &str = r#"{"Version":"2012-10-17","Statement":[]}"#;

/// Data sources code indexes into or feeds to validated arguments: an empty
/// list breaks `names[0]`, a random string is not a UUID.
fn curated(kind: &str) -> Option<Vec<(&'static str, String)>> {
    let s = |v: &str| hcl::label(v);
    let zones = || "[\"us-east-1a\", \"us-east-1b\", \"us-east-1c\"]".to_string();
    Some(match kind {
        "aws_partition" => vec![
            ("partition", s("aws")),
            ("dns_suffix", s("amazonaws.com")),
            ("id", s("aws")),
            ("reverse_dns_prefix", s("com.amazonaws")),
        ],
        "aws_caller_identity" => vec![
            ("account_id", s(ACCOUNT)),
            ("arn", s(&format!("arn:aws:iam::{ACCOUNT}:root"))),
            ("id", s(ACCOUNT)),
            ("user_id", s("MOCK")),
        ],
        "aws_region" => vec![("name", s("us-east-1")), ("region", s("us-east-1")), ("id", s("us-east-1"))],
        "aws_availability_zones" => {
            vec![("names", zones()), ("zone_ids", "[\"use1-az1\", \"use1-az2\", \"use1-az3\"]".to_string())]
        }
        "google_compute_zones" => {
            vec![("names", "[\"us-central1-a\", \"us-central1-b\", \"us-central1-c\"]".to_string())]
        }
        "azurerm_client_config" => {
            vec![("client_id", s(UUID)), ("object_id", s(UUID)), ("subscription_id", s(UUID)), ("tenant_id", s(UUID))]
        }
        "azurerm_subscription" => {
            vec![("id", s(&format!("/subscriptions/{UUID}"))), ("subscription_id", s(UUID)), ("tenant_id", s(UUID))]
        }
        _ => return None,
    })
}

/// An example ID with its placeholders filled in: some docs write
/// `/subscriptions/{subscriptionId}/…`, which no ID parser accepts.
fn filled(id: &str) -> String {
    let mut out = String::new();
    let mut rest = id;
    while let Some((head, tail)) = rest.split_once('{') {
        let Some((name, tail)) = tail.split_once('}') else { break };
        out += head;
        out += &match name {
            n if n.to_lowercase().contains("subscription") => UUID.to_string(),
            "scope" => format!("subscriptions/{UUID}/resourceGroups/mock"),
            _ => "mock".to_string(),
        };
        rest = tail;
    }
    out + rest
}

/// A Compute Engine resource's path: `google_compute_network` gives
/// `projects/mock/global/networks/mock`, `google_compute_subnetwork`
/// `projects/mock/regions/us-central1/subnetworks/mock`.
fn compute_path(kind: &str) -> Option<String> {
    let rest = kind.strip_prefix("google_compute_")?;
    let (global, what) = match rest.strip_prefix("global_") {
        Some(w) => (true, w),
        None => (["network", "firewall", "route"].contains(&rest), rest),
    };
    let plural = if what.ends_with('s') { format!("{what}es") } else { format!("{what}s") };
    let scope = if global { "global".to_string() } else { "regions/us-central1".to_string() };
    Some(format!("projects/mock/{scope}/{plural}/mock"))
}

/// Defaults for one resource or data source type from its schema: only
/// attributes that are computed and not optional, since OpenTofu refuses
/// to mock what the configuration can set.
fn generated(
    provider: &str,
    kind: &str,
    attrs: &serde_json::Map<String, Value>,
    ids: &BTreeMap<&str, &str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, a) in attrs {
        // `id` is optional in SDKv2 schemas, but no configuration can set it.
        let computed_only = a["computed"] == true && (a["optional"] != true || name == "id");
        if !computed_only || a["type"] != "string" {
            continue;
        }
        let value = match provider {
            "aws" if name == "arn" || name.ends_with("_arn") => format!("arn:aws:mock:us-east-1:{ACCOUNT}:{kind}/mock"),
            "aws" if ["policy", "json", "minified_json"].contains(&name.as_str()) || name.ends_with("_policy") => {
                POLICY.to_string()
            }
            "azurerm" if name == "id" => match ids.get(kind) {
                Some(id) => filled(id),
                None => continue,
            },
            // Key Vault keys, secrets and certificates: the ID without its version.
            "azurerm" if name == "versionless_id" => match ids.get(kind).and_then(|id| id.rsplit_once('/')) {
                Some((versionless, _)) => filled(versionless),
                None => continue,
            },
            "azurerm" | "azapi"
                if ["tenant_id", "object_id", "principal_id", "client_id", "subscription_id"]
                    .iter()
                    .any(|s| name == s || name.ends_with(&format!("_{s}"))) =>
            {
                UUID.to_string()
            }
            "google" | "google-beta" if name == "email" => "mock@mock.iam.gserviceaccount.com".to_string(),
            "google" | "google-beta" if name == "member" => {
                "serviceAccount:mock@mock.iam.gserviceaccount.com".to_string()
            }
            // Compute resources are referenced by self link, and validated by its shape.
            "google" | "google-beta" if name == "self_link" => match compute_path(kind) {
                Some(path) => format!("https://www.googleapis.com/compute/v1/{path}"),
                None => continue,
            },
            _ => continue,
        };
        out.push((name.clone(), hcl::label(&value)));
    }
    out
}

/// One provider's `mock_resource` / `mock_data` blocks.
fn profile(provider: &str, schema: &Value) -> String {
    let ids: BTreeMap<&str, &str> =
        AZURERM_IDS.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let mut out = String::new();
    for (key, block) in [("resource_schemas", "mock_resource"), ("data_source_schemas", "mock_data")] {
        let Some(types) = schema[key].as_object() else { continue };
        for (kind, s) in types {
            let attrs = s["block"]["attributes"].as_object().cloned().unwrap_or_default();
            let defaults: Vec<(String, String)> = match (block, curated(kind)) {
                ("mock_data", Some(c)) => c.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
                _ => generated(provider, kind, &attrs, &ids),
            };
            if defaults.is_empty() {
                continue;
            }
            let fields: Vec<String> = defaults.iter().map(|(k, v)| format!("{k} = {v}")).collect();
            out += &format!("  {block} {} {{\n    defaults = {{ {} }}\n  }}\n", hcl::label(kind), fields.join(", "));
        }
    }
    out
}

/// An ARM resource ID of the shape a resource type has:
/// `Microsoft.Network/virtualNetworks/subnets@2024-07-01` gives
/// `/subscriptions/…/resourceGroups/mock/providers/Microsoft.Network/virtualNetworks/mock/subnets/mock`.
fn arm_id(arm_type: &str) -> Option<String> {
    let t = arm_type.split('@').next()?;
    let (namespace, kinds) = t.split_once('/')?;
    if !namespace.contains('.') || kinds.is_empty() {
        return None;
    }
    let rg = format!("/subscriptions/{UUID}/resourceGroups/mock");
    if t.eq_ignore_ascii_case("Microsoft.Resources/resourceGroups") {
        return Some(rg);
    }
    let path: Vec<String> = kinds.split('/').map(|k| format!("{k}/mock")).collect();
    Some(format!("{rg}/providers/{namespace}/{}", path.join("/")))
}

/// A string argument's value when OpenTofu would know it from the module
/// alone: a literal, a variable's literal default, or the default of an
/// `optional(string, "…")` attribute of an object variable.
fn resolved(expr: &str, blocks: &[Block]) -> Option<String> {
    if let Some(v) = hcl::literal(expr) {
        return Some(v);
    }
    let path = expr.strip_prefix("var.")?;
    let (name, attr) = path.split_once('.').map_or((path, None), |(n, a)| (n, Some(a)));
    let var = blocks.iter().find(|b| b.kind == "variable" && b.labels.first().map(String::as_str) == Some(name))?;
    match attr {
        None => var.literal("default"),
        Some(attr) => {
            let ty = var.body.get("type")?.as_str()?;
            let at = ty.find(&format!("{attr} "))? + attr.len();
            let rest = ty[at..].trim_start().strip_prefix('=')?.trim_start().strip_prefix("optional(string,")?;
            let end = rest.find(')')?;
            hcl::literal(rest[..end].trim())
        }
    }
}

/// The attribute paths code reads from `azapi_resource.<name>.output`
/// (`["properties", "defaultHostName"]`), as an HCL object whose leaves are
/// null. Mocked, `output` would be null and every such read fails; a null
/// leaf converts to whatever type the code expects (a name, a set of
/// address prefixes).
fn output_paths(needle: &str, attr: &str, text: &str) -> String {
    /// `iterated`: some `for` loops over this leaf, so it is an empty
    /// object rather than null.
    #[derive(Default)]
    struct Tree(BTreeMap<String, Tree>, bool);
    fn render(t: &Tree) -> String {
        let leaf = |v: &Tree| if v.1 { "{}" } else { "null" }.to_string();
        let fields: Vec<String> =
            t.0.iter().map(|(k, v)| format!("{k} = {}", if v.0.is_empty() { leaf(v) } else { render(v) })).collect();
        if fields.is_empty() { "{}".to_string() } else { format!("{{ {} }}", fields.join(", ")) }
    }
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut tree = Tree::default();
    for (at, _) in text.match_indices(&needle) {
        let mut rest = &text[at + needle.len()..];
        if rest.starts_with(ident) || text[..at].ends_with(|c: char| ident(c) || c == '.') {
            continue; // a longer name, or `data.azapi_resource.…`
        }
        if let Some(r) = rest.strip_prefix('[') {
            rest = r.split_once(']').map_or("", |(_, r)| r);
        }
        let Some(mut rest) = rest.strip_prefix(attr) else { continue };
        let before = text[..at].trim_end();
        let before = before.strip_suffix("try(").or_else(|| before.strip_suffix("coalesce(")).unwrap_or(before);
        let iterated = before.trim_end().ends_with(" in");
        let mut node = &mut tree;
        while let Some(r) = rest.strip_prefix('.') {
            let len = r.find(|c: char| !ident(c)).unwrap_or(r.len());
            if len == 0 {
                break;
            }
            node = node.0.entry(r[..len].to_string()).or_default();
            rest = &r[len..];
        }
        node.1 |= iterated;
    }
    render(&tree)
}

/// `override_resource` targets for every `azapi_resource` in the root
/// module and the modules `tofu init` installed into `dir`: (address, HCL
/// values). azapi resources share one OpenTofu type, so the provider
/// profile cannot give them typed values. Modules validate the IDs they are
/// handed (a subnet's parent must be a virtual network), so each gets an ID
/// of its ARM type's shape when that type is known before planning, and an
/// `output` holding every attribute the module reads from it.
pub fn azapi_overrides(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (module, blocks, all, _) in modules(dir) {
        for b in resources(&blocks, "azapi_resource") {
            let Some(name) = b.labels.get(1) else { continue };
            let ty = b.body.get("type").and_then(Value::as_str).and_then(|t| resolved(t, &blocks));
            let mut values =
                vec![format!("output = {}", output_paths(&format!("azapi_resource.{name}"), ".output", &all))];
            if let Some(id) = ty.as_deref().and_then(arm_id) {
                values.insert(0, format!("id = {}", hcl::label(&id)));
            }
            out.push((format!("{module}azapi_resource.{name}"), format!("{{ {} }}", values.join(", "))));
        }
    }
    out
}

/// `override_resource` targets for google resources whose computed value is
/// validated where it is used, but which the configuration may also set (so
/// the provider profile cannot mock it): an API Gateway API's
/// `managed_service`, which code enables as a service.
pub fn google_overrides(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (module, blocks, _, _) in modules(dir) {
        for b in resources(&blocks, "google_api_gateway_api").filter(|b| !b.body.contains_key("managed_service")) {
            let Some(name) = b.labels.get(1) else { continue };
            out.push((
                format!("{module}google_api_gateway_api.{name}"),
                "{ managed_service = \"mock.apigateway.mock.cloud.goog\" }".to_string(),
            ));
        }
    }
    out
}

/// `override_data` targets for every `terraform_remote_state` in the root
/// module and the modules installed into `dir`, at the top level or in a
/// `check` block. OpenTofu's built-in provider stays real (modules read
/// `terraform_data` outputs), and a real remote state fetches any URL or
/// reads any local file. Its `outputs` hold what code reads. Fails, naming
/// the file, when a file that names it cannot be read here.
pub fn remote_state_overrides(dir: &Path) -> Result<Vec<(String, String)>, String> {
    const PREFIX: &str = "data \"terraform_remote_state\" \"";
    let mut out = Vec::new();
    for (module, blocks, all, unread) in modules(dir) {
        if let Some((file, _)) = unread.iter().find(|(_, t)| t.contains("terraform_remote_state")) {
            return Err(format!(
                "{module}{file}: cannot be read to mock its terraform_remote_state; nothing was planned"
            ));
        }
        let top = blocks
            .iter()
            .filter(|b| b.kind == "data" && b.labels.first().map(String::as_str) == Some("terraform_remote_state"))
            .filter_map(|b| b.labels.get(1).cloned());
        let scoped = blocks.iter().filter(|b| b.kind == "check").flat_map(|b| {
            b.body
                .keys()
                .filter_map(|k| k.strip_prefix(PREFIX)?.strip_suffix('"').map(str::to_string))
                .collect::<Vec<_>>()
        });
        for name in top.chain(scoped) {
            let paths = output_paths(&format!("data.terraform_remote_state.{name}"), ".outputs", &all);
            out.push((format!("{module}data.terraform_remote_state.{name}"), format!("{{ outputs = {paths} }}")));
        }
    }
    Ok(out)
}

fn resources<'b>(blocks: &'b [Block], kind: &'b str) -> impl Iterator<Item = &'b Block> {
    blocks.iter().filter(move |b| b.kind == "resource" && b.labels.first().map(String::as_str) == Some(kind))
}

/// The root module and every module `tofu init` installed into `dir`:
/// (address prefix like `module.a.module.b.`, its blocks, its text, the
/// files that do not parse, with their text).
type Module = (String, Vec<Block>, String, Vec<(String, String)>);
fn modules(dir: &Path) -> Vec<Module> {
    // Without module calls there is no manifest: the root module alone.
    let text = std::fs::read_to_string(dir.join(".terraform/modules/modules.json"))
        .unwrap_or_else(|_| r#"{"Modules":[{"Key":"","Dir":"."}]}"#.into());
    let Ok(json) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    let mut out = Vec::new();
    for m in json["Modules"].as_array().into_iter().flatten() {
        let (Some(key), Some(sub)) = (m["Key"].as_str(), m["Dir"].as_str()) else { continue };
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else { continue };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "tf" || x == "tofu"))
            .collect();
        files.sort();
        let texts: Vec<(String, String)> = files
            .iter()
            .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), std::fs::read_to_string(p).ok()?)))
            .collect();
        let mut blocks = Vec::new();
        let mut unread = Vec::new();
        for (f, t) in &texts {
            match hcl::parse(f, t) {
                Ok(b) => blocks.extend(b),
                Err(_) => unread.push((f.clone(), t.clone())),
            }
        }
        let all: String = texts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join("\n");
        let module: String = key.split('.').filter(|k| !k.is_empty()).map(|k| format!("module.{k}.")).collect();
        out.push((module, blocks, all, unread));
    }
    out
}

/// The test file: a `mock_provider` for every provider in `schema` (the
/// output of `tofu providers schema -json`), under its type and every
/// local name the root module gives it (`locals`: local name, address), and
/// every alias the root module's `blocks` configure, the `overrides`
/// (address, values) from [`azapi_overrides`], then one plan.
pub fn test_file(
    schema: &Value,
    blocks: &[Block],
    locals: &[(String, String)],
    overrides: &[(String, String)],
) -> String {
    let mut out = String::from("# Written by rhizo for an offline plan; never part of a diagram or bundle.\n");
    for (address, s) in schema["provider_schemas"].as_object().into_iter().flatten() {
        // `terraform_data` is OpenTofu itself; its `terraform_remote_state`
        // is overridden instead ([`remote_state_overrides`]).
        if address.starts_with("terraform.io/builtin/") {
            continue;
        }
        let ty = address.rsplit('/').next().unwrap_or(address);
        let body = profile(ty, s);
        // A configuration left unmocked runs the real provider (`external`
        // runs programs), so every local name of this provider gets a mock.
        let mut names = vec![ty];
        for (local, at) in locals {
            if at.eq_ignore_ascii_case(address) && !names.contains(&local.as_str()) {
                names.push(local);
            }
        }
        for name in names {
            let mut aliases: Vec<Option<String>> = blocks
                .iter()
                .filter(|b| b.kind == "provider" && b.labels.first().map(String::as_str) == Some(name))
                .filter_map(|b| b.literal("alias").map(Some))
                .collect();
            aliases.insert(0, None);
            aliases.dedup();
            for alias in aliases {
                out += &format!("\nmock_provider {} {{\n", hcl::label(name));
                if let Some(a) = alias {
                    out += &format!("  alias = {}\n", hcl::label(&a));
                }
                out += &body;
                out += "}\n";
            }
        }
    }
    for (target, values) in overrides {
        let mut rest = target.as_str();
        while let Some(r) = rest.strip_prefix("module.") {
            rest = r.split_once('.').map_or("", |(_, r)| r);
        }
        let block = if rest.starts_with("data.") { "override_data" } else { "override_resource" };
        out += &format!("\n{block} {{\n  target = {target}\n  values = {values}\n}}\n");
    }
    out + "\nrun \"rhizo_l1\" {\n  command = plan\n}\n"
}

/// The plan from `tofu test -json -verbose` output, or what went wrong.
pub fn plan_of(stdout: &str) -> Result<Value, String> {
    let mut plan = None;
    let mut errors = Vec::new();
    for line in stdout.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
        match e["type"].as_str() {
            Some("test_plan") => {
                let mut p = e["test_plan"].clone();
                // In `tofu test` plans (OpenTofu 1.13) this map also holds
                // every module's outputs under bare, colliding names, so its
                // values vary by run; `output_changes` has the root outputs.
                if let Some(v) = p["planned_values"].as_object_mut() {
                    v.remove("outputs");
                }
                plan = Some(p);
            }
            Some("diagnostic") if e["diagnostic"]["severity"] == "error" => {
                let d = &e["diagnostic"];
                let at = match (d["range"]["filename"].as_str(), d["range"]["start"]["line"].as_u64()) {
                    (Some(f), Some(l)) => format!(" ({f}:{l})"),
                    _ => String::new(),
                };
                let detail = d["detail"].as_str().unwrap_or_default();
                errors.push(format!(
                    "{}{at}{}",
                    d["summary"].as_str().unwrap_or("error"),
                    if detail.is_empty() { String::new() } else { format!(": {detail}") }
                ));
            }
            _ => {}
        }
    }
    match plan {
        Some(p) if errors.is_empty() && p["errored"] != true => Ok(p),
        _ if !errors.is_empty() => Err(errors.join("\n")),
        _ => Err("tofu test produced no plan".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn profiles_mock_only_computed_values_in_valid_shapes() {
        let schema = json!({"provider_schemas": {
            "registry.opentofu.org/hashicorp/aws": {
                "resource_schemas": {"aws_sns_topic": {"block": {"attributes": {
                    "arn": {"type": "string", "computed": true},
                    "name": {"type": "string", "optional": true, "computed": true},
                    "policy": {"type": "string", "optional": true, "computed": true}
                }}}},
                "data_source_schemas": {"aws_availability_zones": {"block": {"attributes": {}}}}
            },
            "registry.opentofu.org/hashicorp/azurerm": {
                "resource_schemas": {"azurerm_storage_account": {"block": {"attributes": {
                    "id": {"type": "string", "computed": true}
                }}}}
            },
            "registry.opentofu.org/hashicorp/google": {},
            "terraform.io/builtin/terraform": {}
        }});
        let alias = crate::hcl::parse("p.tf", "provider \"aws\" {\n  alias = \"eu\"\n}\n").unwrap();
        let f = test_file(&schema, &alias, &[], &[]);
        assert!(f.contains("arn = \"arn:aws:mock:us-east-1:123456789012:aws_sns_topic/mock\""), "{f}");
        assert!(!f.contains("name =") && !f.contains("policy ="), "settable values are not mocked: {f}");
        assert!(f.contains("names = [\"us-east-1a\""), "{f}");
        assert!(f.contains("/providers/Microsoft.Storage/storageAccounts/"), "{f}");
        assert_eq!(f.matches("mock_provider \"aws\"").count(), 2, "default and alias: {f}");
        assert!(f.contains("alias = \"eu\"") && f.contains("mock_provider \"google\" {\n}"), "{f}");
        assert!(!f.contains("mock_provider \"terraform\""), "built-ins stay real: {f}");
        assert!(f.ends_with("run \"rhizo_l1\" {\n  command = plan\n}\n"));
    }

    /// Every remote state is overridden, top-level or in a `check` block,
    /// in the root module or a module; an unreadable file naming one is refused.
    #[test]
    fn every_remote_state_is_overridden() {
        let dir = std::env::temp_dir().join(format!("rhizo-remote-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".terraform/modules/m")).unwrap();
        std::fs::write(
            dir.join(".terraform/modules/modules.json"),
            r#"{"Modules":[{"Key":"","Dir":"."},{"Key":"m","Dir":".terraform/modules/m"}]}"#,
        )
        .unwrap();
        let root = "data \"terraform_remote_state\" \"net\" {\n  backend = \"http\"\n}\nlocals {\n  v = data.terraform_remote_state.net.outputs.vpc.id\n}\ncheck \"c\" {\n  data \"terraform_remote_state\" \"scoped\" {\n    backend = \"local\"\n  }\n  assert {\n    condition     = data.terraform_remote_state.scoped.outputs != null\n    error_message = \"x\"\n  }\n}\n";
        std::fs::write(dir.join("main.tf"), root).unwrap();
        std::fs::write(
            dir.join(".terraform/modules/m/main.tf"),
            "data \"terraform_remote_state\" \"s\" {\n  backend = \"s3\"\n}\n",
        )
        .unwrap();
        let o = remote_state_overrides(&dir).unwrap();
        let targets: Vec<&str> = o.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            targets,
            [
                "data.terraform_remote_state.net",
                "data.terraform_remote_state.scoped",
                "module.m.data.terraform_remote_state.s"
            ]
        );
        assert_eq!(o[0].1, "{ outputs = { vpc = { id = null } } }");
        let f = test_file(&json!({}), &[], &[], &o);
        assert_eq!(f.matches("\noverride_data {\n  target = ").count(), 3, "{f}");
        std::fs::write(
            dir.join(".terraform/modules/m/bad.tf"),
            "data \"terraform_remote_state\" \"t\" {\n  a = 1\n  a = 2\n}\n",
        )
        .unwrap();
        let e = remote_state_overrides(&dir).unwrap_err();
        assert!(e.contains("module.m.bad.tf"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_local_name_of_a_provider_is_mocked() {
        let schema = json!({"provider_schemas": {"registry.opentofu.org/hashicorp/external": {}}});
        let blocks = crate::hcl::parse("p.tf", "provider \"ext\" {\n  alias = \"a\"\n}\n").unwrap();
        let locals = [("ext".to_string(), "registry.opentofu.org/hashicorp/external".to_string())];
        let f = test_file(&schema, &blocks, &locals, &[]);
        assert!(f.contains("mock_provider \"external\" {\n}"), "{f}");
        assert!(
            f.contains("mock_provider \"ext\" {\n}") && f.contains("mock_provider \"ext\" {\n  alias = \"a\""),
            "{f}"
        );
    }

    #[test]
    fn azapi_resources_get_ids_of_their_arm_type_and_outputs() {
        assert_eq!(
            arm_id("Microsoft.Network/virtualNetworks/subnets@2024-07-01").unwrap(),
            format!(
                "/subscriptions/{UUID}/resourceGroups/mock/providers/Microsoft.Network/virtualNetworks/mock/subnets/mock"
            )
        );
        assert_eq!(
            arm_id("Microsoft.Resources/resourceGroups@2025-04-01").unwrap(),
            format!("/subscriptions/{UUID}/resourceGroups/mock")
        );
        assert_eq!(arm_id("each.value.type"), None);
        assert_eq!(
            filled("/subscriptions/{subscriptionId}/resourceGroups/{rg}/x"),
            format!("/subscriptions/{UUID}/resourceGroups/mock/x")
        );

        let dir = std::env::temp_dir().join(format!("rhizo-azapi-{}", std::process::id()));
        let module = dir.join(".terraform/modules/st");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(
            dir.join(".terraform/modules/modules.json"),
            r#"{"Modules":[{"Key":"","Source":"","Dir":"."},{"Key":"st","Dir":".terraform/modules/st"}]}"#,
        )
        .unwrap();
        std::fs::write(
            module.join("main.tf"),
            r#"variable "resource_types" {
  type = object({
    storage_account = optional(string, "Microsoft.Storage/storageAccounts@2025-08-01")
  })
  default = {}
}

resource "azapi_resource" "this" {
  type = var.resource_types.storage_account
}

resource "azapi_resource" "lock" {
  type = local.lock_type
}

locals {
  hosts = { for k, v in try(azapi_resource.this.output.properties.primaryEndpoints, {}) : k => v }
  name  = azapi_resource.this.output.name
}
"#,
        )
        .unwrap();
        let o = azapi_overrides(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            o,
            [
                (
                    "module.st.azapi_resource.this".to_string(),
                    format!(
                        "{{ id = \"/subscriptions/{UUID}/resourceGroups/mock/providers/Microsoft.Storage/storageAccounts/mock\", output = {{ name = null, properties = {{ primaryEndpoints = {{}} }} }} }}"
                    )
                ),
                ("module.st.azapi_resource.lock".to_string(), "{ output = {} }".to_string()),
            ]
        );
        let f = test_file(&json!({}), &[], &[], &o);
        assert!(f.contains("override_resource {\n  target = module.st.azapi_resource.this\n  values = { id = "), "{f}");
    }

    #[test]
    fn google_values_have_valid_shapes() {
        assert_eq!(compute_path("google_compute_network").unwrap(), "projects/mock/global/networks/mock");
        assert_eq!(compute_path("google_compute_global_address").unwrap(), "projects/mock/global/addresses/mock");
        assert_eq!(
            compute_path("google_compute_subnetwork").unwrap(),
            "projects/mock/regions/us-central1/subnetworks/mock"
        );
        assert_eq!(compute_path("google_storage_bucket"), None);
        let schema = json!({"provider_schemas": {"registry.opentofu.org/hashicorp/google-beta": {
            "resource_schemas": {"google_compute_network": {"block": {"attributes": {
                "self_link": {"type": "string", "computed": true},
                "name": {"type": "string", "required": true}
            }}}},
            "data_source_schemas": {"google_compute_zones": {"block": {"attributes": {}}}}
        }}});
        let f = test_file(&schema, &[], &[], &[]);
        assert!(
            f.contains("self_link = \"https://www.googleapis.com/compute/v1/projects/mock/global/networks/mock\""),
            "{f}"
        );
        assert!(f.contains("names = [\"us-central1-a\""), "{f}");

        let dir = std::env::temp_dir().join(format!("rhizo-google-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".terraform/modules")).unwrap();
        std::fs::write(dir.join(".terraform/modules/modules.json"), r#"{"Modules":[{"Key":"","Dir":"."}]}"#).unwrap();
        std::fs::write(
            dir.join("main.tf"),
            "resource \"google_api_gateway_api\" \"a\" {\n  api_id = \"a\"\n}\n\nresource \"google_api_gateway_api\" \"b\" {\n  api_id          = \"b\"\n  managed_service = \"b.apigateway.p.cloud.goog\"\n}\n",
        )
        .unwrap();
        let o = google_overrides(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            o,
            [(
                "google_api_gateway_api.a".to_string(),
                "{ managed_service = \"mock.apigateway.mock.cloud.goog\" }".to_string()
            )],
            "a configured value is never overridden"
        );
    }

    #[test]
    fn plan_of_reads_the_plan_or_the_errors() {
        let ok = "{\"type\":\"version\"}\n{\"type\":\"test_plan\",\"test_plan\":{\"planned_values\":{\"outputs\":{\"x\":1}},\"errored\":false}}\n";
        assert_eq!(plan_of(ok).unwrap()["planned_values"], json!({}), "module outputs leak into this map");
        let bad = "{\"type\":\"diagnostic\",\"diagnostic\":{\"severity\":\"error\",\"summary\":\"Invalid ARN\",\"detail\":\"x\",\"range\":{\"filename\":\"main.tf\",\"start\":{\"line\":3}}}}\n";
        assert_eq!(plan_of(bad).unwrap_err(), "Invalid ARN (main.tf:3): x");
    }
}
