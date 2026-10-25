//! Raw Azure and Google Cloud resources: which ones adopting refuses and
//! why, the provider version their code is on, and the settings rhizo's
//! code would take over only when they are exactly what it writes (the
//! offline plan cannot compare what they grant).

use rhizo_core::{Frame, Node};
use serde_json::Value;

/// Raw resources rhizo's code cannot take over faithfully, and why. Each
/// reason was checked against the pinned module or provider source (see
/// docs/design/iac-import.md, "Adopt", coverage).
const NOT_YET: &[(&str, &str)] = &[
    (
        "azurerm_storage_account",
        "rhizo's code for it is the storage account AVM module, built on azapi: moving `azurerm_storage_account` into `azapi_resource` is a move across types, which azapi 2.13 implements, but OpenTofu 1.13 carries the azurerm resource identity over and then fails to plan (`attributes \"id\" and \"type\" are required`), and the moved state holds only the id until a live refresh",
    ),
    (
        "azurerm_virtual_network",
        "rhizo's code for it is the virtual network AVM module, built on azapi: a move across types, where OpenTofu 1.13 carries the azurerm resource identity over and fails to plan",
    ),
    (
        "azurerm_subnet",
        "rhizo's code writes subnets as tiers inside the virtual network AVM module, built on azapi: a move across types, where OpenTofu 1.13 carries the azurerm resource identity over and fails to plan",
    ),
    (
        "azurerm_linux_function_app",
        "rhizo's code for a function is the web site AVM module, built on azapi (a move across types, where OpenTofu 1.13 fails on the carried resource identity), with its own plan, identity, host storage and Application Insights",
    ),
    (
        "azurerm_function_app_flex_consumption",
        "rhizo's code for a function is the web site AVM module, built on azapi (a move across types, whose moved state holds only the id until a live refresh), with its own plan, identity, host storage and Application Insights",
    ),
    (
        "azurerm_servicebus_queue",
        "rhizo's code puts every queue and topic of an environment into one Service Bus namespace module it names after the environment; this queue's namespace is code of its own",
    ),
    (
        "azurerm_servicebus_topic",
        "rhizo's code puts every queue and topic of an environment into one Service Bus namespace module it names after the environment; this topic's namespace is code of its own",
    ),
    (
        "azurerm_cosmosdb_account",
        "rhizo's Cosmos DB code is a serverless account (`EnableServerless`, which an existing provisioned account cannot take on) with account keys turned off (`local_authentication_disabled`), and names its database and container after the node",
    ),
    (
        "azurerm_postgresql_flexible_server",
        "rhizo's PostgreSQL code is Entra ID only, with an admin group from variables this environment does not declare, and turns password authentication and public network access off: the logins that exist would stop working",
    ),
    (
        "azurerm_api_management",
        "rhizo's API Management code writes the APIs, operations and policies inside its module call; here they are resources of their own",
    ),
    (
        "google_pubsub_subscription",
        "rhizo's code for a queue is a topic of its own with one subscription and a dead-letter topic; this subscription reads a topic that is other code",
    ),
    (
        "google_sql_database_instance",
        "rhizo's Cloud SQL code (the sql-db postgresql module) runs the instance on provider google-beta, adds IAM database users and an admin group from variables this environment does not declare, and always creates a default database and a null_resource",
    ),
    (
        "google_compute_network",
        "the network module rhizo's code calls owns its subnets, routes and firewall rules: a different set of resources",
    ),
    (
        "google_compute_subnetwork",
        "rhizo's code writes subnets as tiers inside the network module, which owns their routes and firewall rules",
    ),
];

/// Types rhizo's own Google code writes as raw resources whose code needs
/// rhizo's environment around it.
const NEEDS_RHIZOS_ENV: &[(&str, &str)] = &[
    (
        "google_cloudfunctions2_function",
        "rhizo's function code builds from its function sources bucket and runs as a service account of its service-accounts module; this environment has neither",
    ),
    (
        "google_api_gateway_gateway",
        "rhizo's API gateway code serves an OpenAPI document of its routes and runs as a service account of its service-accounts module; this environment has neither",
    ),
];

/// Why a raw resource of type `ty` cannot be adopted, if it cannot.
/// `rhizos_env`: the environment is rhizo's own output (its helpers exist).
pub(super) fn not_yet(ty: &str, rhizos_env: bool) -> Option<&'static str> {
    let own = NEEDS_RHIZOS_ENV.iter().filter(|_| !rhizos_env);
    NOT_YET.iter().chain(own).find(|(t, _)| *t == ty).map(|(_, why)| *why)
}

/// The locals the environment's code defines.
pub(super) fn locals(env: &Frame) -> Vec<&str> {
    blocks(env)
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("locals"))
        .flat_map(|b| b.get("body").and_then(Value::as_object).into_iter().flatten().map(|(k, _)| k.as_str()))
        .collect()
}

fn blocks(env: &Frame) -> impl Iterator<Item = &Value> {
    env.props.get("blocks").and_then(Value::as_array).into_iter().flatten()
}

/// An argument of the environment's default `provider "<name>"` block.
pub(super) fn provider_arg<'a>(env: &'a Frame, name: &str, arg: &str) -> Option<&'a str> {
    blocks(env)
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("provider"))
        .filter(|b| b.get("labels").and_then(|l| l.get(0)).and_then(Value::as_str) == Some(name))
        .filter(|b| b.get("body").and_then(|x| x.get("alias")).is_none())
        .find_map(|b| b.get("body")?.get(arg)?.as_str())
}

/// The provider rhizo's code for a cloud is pinned to, and the range of
/// versions the modules it calls accept.
fn pin(cloud: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match cloud {
        // The pubsub module needs < 8; rhizo pins the newest 7.x.
        "gcp" => Some(("google", "7.46.1", ">= 7.0.0, < 8.0.0")),
        // The key vault module needs >= 4.81, < 5.0.
        "azure" => Some(("azurerm", "4.81.0", ">= 4.81.0, < 5.0.0")),
        _ => None,
    }
}

/// Refuses when the environment's code is on a provider version outside
/// rhizo's pin: its `required_providers` constraint must admit the pinned
/// version, and its lock file must lock one the modules accept.
pub(super) fn provider_version(env: &Frame, cloud: &str, address: &str) -> Result<(), String> {
    let Some((name, pinned, range)) = pin(cloud) else { return Ok(()) };
    let source = format!("hashicorp/{name}");
    let cannot = |why: String| {
        format!("cannot adopt `{address}`: rhizo's code is pinned to {source} {pinned}, and this environment {why}")
    };
    for b in blocks(env).filter(|b| b.get("type").and_then(Value::as_str) == Some("terraform")) {
        let required = b.get("body").and_then(|x| x.get("required_providers")).and_then(Value::as_array);
        for r in required.into_iter().flatten() {
            let Some(text) = r.get(name).and_then(Value::as_str) else { continue };
            if let Some(c) = quoted_after(text, "version")
                && !admits(&c, pinned)
            {
                return Err(cannot(format!("requires {name} `{c}`")));
            }
        }
    }
    if let Some(lock) = env.props.get("lock-file").and_then(Value::as_str)
        && let Some(v) = locked(lock, &source)
        && !admits(range, &v)
    {
        return Err(cannot(format!("locks {name} {v} (rhizo's modules need {range}; upgrade it first)")));
    }
    Ok(())
}

/// The version a lock file locks `source` (`hashicorp/google`) at.
fn locked(lock: &str, source: &str) -> Option<String> {
    let at = lock.find(&format!("/{source}\""))?;
    quoted_after(&lock[at..], "version")
}

/// The string after `key = ` in `text`.
fn quoted_after(text: &str, key: &str) -> Option<String> {
    let mut rest = text;
    while let Some(i) = rest.find(key) {
        let after = rest[i + key.len()..].trim_start();
        let before_ok = rest[..i].chars().last().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
        if let (true, Some(v)) = (before_ok, after.strip_prefix('=')) {
            let v = v.trim_start().strip_prefix('"')?;
            return Some(v[..v.find('"')?].to_string());
        }
        rest = &rest[i + key.len()..];
    }
    None
}

/// Whether version constraint `c` (`~> 7.0`, `>= 6.2, < 8`, `= 7.46.1`)
/// admits `version`. Unreadable parts admit nothing.
fn admits(c: &str, version: &str) -> bool {
    let num =
        |s: &str| -> Option<Vec<u64>> { s.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect() };
    let Some(v) = num(version) else { return false };
    let cmp = |want: &[u64]| {
        let pad = |x: &[u64], i: usize| x.get(i).copied().unwrap_or(0);
        (0..3).map(|i| pad(&v, i).cmp(&pad(want, i))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
    };
    c.split(',').all(|part| {
        let part = part.trim();
        let (op, rest) = ["~>", ">=", "<=", "!=", ">", "<", "="]
            .iter()
            .find_map(|op| part.strip_prefix(op).map(|r| (*op, r)))
            .unwrap_or(("=", part));
        let Some(want) = num(rest) else { return false };
        use std::cmp::Ordering::*;
        match op {
            "~>" => {
                // The last given part may grow: `~> 7.0` is >= 7.0, < 8.0.
                let mut top = want[..want.len().saturating_sub(1).max(1)].to_vec();
                if let Some(l) = top.last_mut() {
                    *l += 1;
                }
                cmp(&want) != Less && cmp(&top) == Less
            }
            ">=" => cmp(&want) != Less,
            "<=" => cmp(&want) != Greater,
            ">" => cmp(&want) == Greater,
            "<" => cmp(&want) == Less,
            "!=" => cmp(&want) != Equal,
            _ => cmp(&want) == Equal,
        }
    })
}

/// A key vault rhizo's code would take over: RBAC only (rhizo's code turns
/// access policies off), and network rules no narrower than the Allow rhizo
/// writes. Access policies and network rules grant what the offline plan
/// cannot compare, so anything else is refused by name.
pub(super) fn vault_is_rhizos(code: &[&Node], vault: &Node, address: &str) -> Result<(), String> {
    let at = vault.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str).unwrap_or_default();
    let cannot = |why: &str| {
        format!(
            "cannot adopt `{address}`: its key vault `{at}` {why}, and rhizo's code turns that off, so grants would be lost unseen"
        )
    };
    let body = vault.props.get("body").and_then(Value::as_object);
    let arg = |k: &str| body.and_then(|b| b.get(k)).and_then(Value::as_str).map(str::trim);
    if arg("rbac_authorization_enabled").or(arg("enable_rbac_authorization")) != Some("true") {
        return Err(cannot("authorizes with access policies (RBAC is not on)"));
    }
    if body.is_some_and(|b| b.contains_key("access_policy")) {
        return Err(cannot("has access policies"));
    }
    let policies = code
        .iter()
        .filter(|n| n.props.get("tf-type").and_then(Value::as_str) == Some("azurerm_key_vault_access_policy"));
    if let Some(p) =
        policies.into_iter().find(|p| p.props.get("body").is_some_and(|b| b.to_string().contains(&format!("{at}."))))
    {
        let p = p.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str).unwrap_or_default();
        return Err(cannot(&format!("has access policy `{p}`")));
    }
    // rhizo writes `network_acls = { default_action = "Allow", bypass = "AzureServices" }`.
    match body.and_then(|b| b.get("network_acls")) {
        None => Ok(()),
        Some(Value::Array(acls)) if acls.len() == 1 => {
            let acl = acls[0].as_object();
            let get = |k: &str| acl.and_then(|a| a.get(k)).and_then(Value::as_str).map(str::trim);
            let empty = |k: &str| get(k).is_none_or(|v| v == "[]");
            if get("default_action") == Some("\"Allow\"")
                && get("bypass") == Some("\"AzureServices\"")
                && empty("ip_rules")
                && empty("virtual_network_subnet_ids")
                && acl.is_some_and(|a| a.len() <= 4)
            {
                Ok(())
            } else {
                Err(format!(
                    "cannot adopt `{address}`: its key vault `{at}` has network rules other than the Allow rhizo's code writes, which the offline plan cannot compare"
                ))
            }
        }
        Some(_) => Err(format!(
            "cannot adopt `{address}`: its key vault `{at}` has network rules other than the Allow rhizo's code writes, which the offline plan cannot compare"
        )),
    }
}

/// A Cloud Storage bucket rhizo's code would take over: rhizo turns on
/// uniform bucket-level access (which ends every ACL grant) and enforced
/// public access prevention (which ends grants to allUsers). Taken over
/// only when the bucket has the first already and nothing grants the public.
pub(super) fn bucket_is_rhizos(code: &[&Node], bucket: &Node, address: &str) -> Result<(), String> {
    let body = bucket.props.get("body").and_then(Value::as_object);
    let arg = |k: &str| body.and_then(|b| b.get(k)).and_then(Value::as_str).map(str::trim);
    if arg("uniform_bucket_level_access") != Some("true") {
        return Err(format!(
            "cannot adopt `{address}`: rhizo's bucket code turns on uniform bucket-level access, which ends every ACL grant on the bucket and its objects, and this bucket's code does not have it on"
        ));
    }
    if arg("public_access_prevention") == Some("\"enforced\"") {
        return Ok(());
    }
    let public = code.iter().find(|n| {
        let text = n.props.get("body").map(Value::to_string).unwrap_or_default();
        text.contains(&format!("{address}.")) && (text.contains("allUsers") || text.contains("allAuthenticatedUsers"))
    });
    match public {
        Some(n) => Err(format!(
            "cannot adopt `{address}`: rhizo's bucket code enforces public access prevention, which ends the public grant of `{}`",
            n.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str).unwrap_or_default()
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_constraints() {
        assert!(admits("~> 7.0", "7.46.1"));
        assert!(!admits("~> 7.0", "8.0.0"));
        assert!(admits("~> 7.40.0", "7.40.9") && !admits("~> 7.40.0", "7.41.0"));
        assert!(admits(">= 6.2, < 8", "7.46.1"));
        assert!(!admits(">= 6.2, < 7", "7.46.1"));
        assert!(admits("= 4.81.0", "4.81.0") && admits("4.81.0", "4.81.0") && !admits("4.80.0", "4.81.0"));
        assert!(!admits(">= 4.81.0, < 5.0.0", "4.80.0"));
        assert!(!admits("latest", "7.46.1"), "unreadable admits nothing");
        let lock = "provider \"registry.opentofu.org/hashicorp/google\" {\n  version     = \"6.49.0\"\n  constraints = \"~> 6.0\"\n}\n";
        assert_eq!(locked(lock, "hashicorp/google").as_deref(), Some("6.49.0"));
        assert_eq!(locked(lock, "hashicorp/azurerm"), None);
        assert_eq!(
            quoted_after("{\n source = \"hashicorp/google\"\n version = \"~> 7.0\"\n}", "version").as_deref(),
            Some("~> 7.0")
        );
    }

    #[test]
    fn refusals_name_the_type() {
        assert!(not_yet("azurerm_storage_account", false).is_some_and(|w| w.contains("azapi")));
        assert!(not_yet("google_cloudfunctions2_function", false).is_some());
        assert_eq!(not_yet("google_cloudfunctions2_function", true), None, "rhizo's own output adopts");
        assert_eq!(not_yet("google_storage_bucket", false), None);
    }
}
