//! The Azure compiler for the cloud-neutral `infra.*` types: an environment
//! with `cloud="azure"` becomes one OpenTofu root module of Azure Verified
//! Modules (docs/design/infra-cloud-neutral.md, step 3).
//!
//! - `versions.tf`: OpenTofu and the providers pinned exactly, the azurerm
//!   backend with Entra ID auth.
//! - `providers.tf`, `main.tf` (one resource group, AVM modules pinned to
//!   exact versions), `iam.tf` (a user-assigned identity per function and
//!   least-privilege role assignments from edges), `network.tf` (network
//!   security groups for database subnets), `variables.tf` (ids and people,
//!   without defaults) and `outputs.tf`.
//!
//! What has no faithful Azure mapping is a finding of the kit rules
//! (`rhizo_kit::infra::azure::gaps`), never dropped here.

use std::collections::BTreeSet;

use rhizo_core::{Document, Edge, Frame, Node};
use rhizo_kit::infra::azure::{self as az, DEFAULT_SKU};
use rhizo_kit::infra::{self as k, BUCKET, FUNCTION, HTTP_API, KEY, Link, POSTGRES, QUEUE, SUBNET, TABLE, TOPIC, VPC};
use serde_json::Value;

use super::{Item, Val, a, addr, block, comment, dir, file, generic, list, name, obj, q, upper};
use crate::brief::Criterion;
use crate::{Files, OutFile};

/// The catalog: exact versions, checked against registry.opentofu.org and
/// each module's `terraform.tf` on 2026-10-22 (latest release of each).
/// `tofu init` resolves every module's provider ranges to these.
const AZURERM: &str = "4.81.0"; // the key vault module needs >= 4.81, < 5.0
const AZAPI: &str = "2.13.0";
const MODTM: &str = "0.4.0"; // AVM telemetry; every module requires it, rhizo turns it off
const RANDOM: &str = super::RANDOM;
const TIME: &str = "0.14.2";

const RESOURCE_GROUP: (&str, &str) = ("Azure/avm-res-resources-resourcegroup/azurerm", "0.4.1");
const VNET: (&str, &str) = ("Azure/avm-res-network-virtualnetwork/azurerm", "0.22.3");
const NAT: (&str, &str) = ("Azure/avm-res-network-natgateway/azurerm", "0.3.2");
const NSG: (&str, &str) = ("Azure/avm-res-network-networksecuritygroup/azurerm", "0.6.0");
const DNS: (&str, &str) = ("Azure/avm-res-network-privatednszone/azurerm", "0.5.0");
const STORAGE: (&str, &str) = ("Azure/avm-res-storage-storageaccount/azurerm", "0.10.0");
const COSMOS: (&str, &str) = ("Azure/avm-res-documentdb-databaseaccount/azurerm", "0.11.0");
const POSTGRES_MODULE: (&str, &str) = ("Azure/avm-res-dbforpostgresql-flexibleserver/azurerm", "0.2.3");
const SERVICE_BUS: (&str, &str) = ("Azure/avm-res-servicebus-namespace/azurerm", "0.4.0");
const KEY_VAULT: (&str, &str) = ("Azure/avm-res-keyvault-vault/azurerm", "0.12.0");
const IDENTITY: (&str, &str) = ("Azure/avm-res-managedidentity-userassignedidentity/azurerm", "0.5.3");
const PLAN: (&str, &str) = ("Azure/avm-res-web-serverfarm/azurerm", "2.0.8");
const SITE: (&str, &str) = ("Azure/avm-res-web-site/azurerm", "0.24.1");
const APIM: (&str, &str) = ("Azure/avm-res-apimanagement-service/azurerm", "0.9.0");
const WORKSPACE: (&str, &str) = ("Azure/avm-res-operationalinsights-workspace/azurerm", "0.5.1");
const INSIGHTS: (&str, &str) = ("Azure/avm-res-insights-component/azurerm", "0.5.0");

/// PostgreSQL Flexible Server storage sizes in MB.
const PG_STORAGE_MB: [i64; 12] = [
    32768, 65536, 131_072, 262_144, 524_288, 1_048_576, 2_097_152, 4_193_280, 4_194_304, 8_388_608, 16_777_216,
    33_553_408,
];

/// Built-in data-plane roles.
const BLOB_READER: &str = "Storage Blob Data Reader";
const BLOB_OWNER: &str = "Storage Blob Data Owner";
const SB_SENDER: &str = "Azure Service Bus Data Sender";
const SB_RECEIVER: &str = "Azure Service Bus Data Receiver";
const KEY_USER: &str = "Key Vault Crypto Service Encryption User";
const METRICS_PUBLISHER: &str = "Monitoring Metrics Publisher";
const COSMOS_READER: &str = "00000000-0000-0000-0000-000000000001";
const COSMOS_CONTRIBUTOR: &str = "00000000-0000-0000-0000-000000000002";

const BLOB_ACTIONS: &str = "Microsoft.Storage/storageAccounts/blobServices/containers/blobs";

/// The app setting prefix of the Service Bus connection functions use.
pub(super) const SB_CONNECTION: &str = "SERVICEBUS";

/// The blob container of a bucket's storage account.
const CONTAINER: &str = "data";

/// `module.x`
fn m(n: &Node) -> String {
    format!("module.{}", addr(n.id.as_str()))
}

fn module_block(id: &str, (source, version): (&str, &str), body: Vec<Item>, tags_of: &str) -> Item {
    let mut items = vec![a("source", q(source)), a("version", q(version)), Item::Blank];
    items.extend(body);
    items.extend([Item::Blank, a("tags", format!("merge(local.tags, {{ \"rhizo:node\" = {} }})", q(tags_of)))]);
    block(format!("module \"{id}\""), items)
}

/// `orders-db` in environment `prod` → `prodordersdb`, cut to `max`.
fn compact(s: &str, max: usize) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).take(max).collect()
}

/// `prod-orders-db`, cut to `max` without a trailing hyphen.
fn cut(s: &str, max: usize) -> String {
    let c: String = s.chars().take(max).collect();
    c.trim_end_matches('-').to_string()
}

/// One azure environment.
pub(super) struct Az<'a> {
    doc: &'a Document,
    frame: &'a Frame,
    /// `prod`: part of every Azure name.
    name: String,
    location: String,
    /// `shop` for `shop.rhizo`.
    source: String,
    header: String,
    nodes: Vec<&'a Node>,
}

impl<'a> Az<'a> {
    pub(super) fn new(doc: &'a Document, frame: &'a Frame, source_name: &str) -> Self {
        let header = format!(
            "@generated by rhizo {} from {source_name} (environment {}). Do not edit: change the diagram and recompile.",
            crate::VERSION,
            frame.id
        );
        let nodes = doc.nodes.values().filter(|n| n.frame == frame.id && k::is_infra(k::ty(n))).collect();
        let location = frame.props.get("region").and_then(Value::as_str).unwrap_or("westeurope").to_string();
        let source = name(source_name.trim_end_matches(".rhizo").trim_end_matches(".kdl"));
        Az { doc, frame, name: name(frame.id.as_str()), location, source, header, nodes }
    }

    fn of(&self, ty: &str) -> impl Iterator<Item = &'a Node> + '_ {
        let ty = ty.to_string();
        self.nodes.iter().copied().filter(move |n| k::ty(n) == ty)
    }

    fn any(&self, ty: &str) -> bool {
        self.of(ty).next().is_some()
    }

    fn links(&self) -> Vec<(&'a Edge, &'a Node, &'a Node, Link)> {
        k::links(self.doc)
            .filter(|(_, _, a, b, _)| a.frame == self.frame.id && b.frame == self.frame.id)
            .map(|(_, e, a, b, l)| (e, a, b, l))
            .collect()
    }

    fn links_from(&self, n: &Node, kind: Link) -> Vec<(&'a Edge, &'a Node)> {
        self.links().into_iter().filter(|(_, a, _, l)| a.id == n.id && *l == kind).map(|(e, _, b, _)| (e, b)).collect()
    }

    fn links_to(&self, n: &Node, kind: Link) -> Vec<(&'a Edge, &'a Node)> {
        self.links().into_iter().filter(|(_, _, b, l)| b.id == n.id && *l == kind).map(|(e, a, _, _)| (e, a)).collect()
    }

    /// `prod-orders-db`.
    fn base(&self, n: &Node) -> String {
        format!("{}-{}", self.name, name(n.id.as_str()))
    }

    /// A globally unique name as an HCL expression:
    /// `"prefix-<base>-${local.suffix}"` within `max` characters.
    fn global(&self, prefix: &str, n: &Node, max: usize) -> String {
        let room = max - prefix.len() - 2 - 6;
        format!("\"{prefix}-{}-${{local.suffix}}\"", cut(&self.base(n), room))
    }

    /// A storage account name as an HCL expression: lowercase letters and
    /// digits, at most 24.
    fn storage_name(&self, n: &Node, kind: &str) -> String {
        let room = 24 - 2 - kind.len() - 6;
        format!("\"st{}{kind}${{local.suffix}}\"", compact(&self.base(n), room))
    }

    fn resource_group(&self) -> String {
        let own = self.frame.props.get("azure").and_then(|b| b.get("resource-group")).and_then(Value::as_str);
        own.map_or_else(|| format!("rg-{}-{}", self.source, self.name), str::to_string)
    }

    fn service_bus(&self) -> bool {
        self.any(QUEUE) || self.any(TOPIC)
    }

    /// Keys some store is encrypted with: they get an identity the stores
    /// use to reach them.
    fn used_keys(&self) -> Vec<&'a Node> {
        self.of(KEY)
            .filter(|key| self.nodes.iter().any(|n| k::key(self.doc, n).is_some_and(|x| x.id == key.id)))
            .collect()
    }

    /// Function and database nodes in each subnet tier of `net`.
    fn tenants(&self, net: &Node, tier: &str) -> Vec<&'a Node> {
        az::tenants(self.doc, self.frame)
            .into_iter()
            .filter(|(v, t, _)| v.id == net.id && *t == tier)
            .flat_map(|(_, _, ns)| ns)
            .collect()
    }

    pub(super) fn emit(&self, files: &mut Files) {
        let ids = |ns: &[&Node]| -> Vec<String> {
            let mut v: Vec<String> = ns.iter().map(|n| n.id.to_string()).collect();
            v.sort();
            v
        };
        let all = ids(&self.nodes);
        let mut put = |f: &str, contents: String, nodes: Vec<String>| {
            files.insert(format!("{}/{f}", dir(self.frame)), OutFile { contents, nodes });
        };
        put("versions.tf", self.versions(), Vec::new());
        put("providers.tf", self.providers(), Vec::new());
        put("main.tf", self.main(), all.clone());
        let (iam, iam_nodes) = self.iam();
        if !iam.is_empty() {
            put("iam.tf", file(&self.header, iam), iam_nodes);
        }
        let (net, net_nodes) = self.network();
        if !net.is_empty() {
            put("network.tf", file(&self.header, net), net_nodes);
        }
        let dbs: Vec<&Node> = self.of(POSTGRES).chain(self.of(HTTP_API)).collect();
        put(
            "variables.tf",
            file(&self.header, self.variable_blocks().into_iter().map(|(_, b)| vec![b]).collect()),
            ids(&dbs),
        );
        put("outputs.tf", self.outputs(), all);
    }

    fn versions(&self) -> String {
        let provider =
            |p: &str, source: &str, v: &str| obj(p, vec![a("source", q(source)), a("version", q(&format!("= {v}")))]);
        let providers = vec![
            provider("azapi", "azure/azapi", AZAPI),
            provider("azurerm", "hashicorp/azurerm", AZURERM),
            provider("modtm", "azure/modtm", MODTM),
            provider("random", "hashicorp/random", RANDOM),
            provider("time", "hashicorp/time", TIME),
        ];
        let mut items = vec![
            a("required_version", q(super::OPENTOFU)),
            Item::Blank,
            comment("Exact versions from the infra kit's catalog; upgrading the kit is the only way they change."),
            comment("Every Azure Verified Module requires azapi, modtm and random; some require azurerm and time."),
            block("required_providers", providers),
        ];
        if let Some((account, container, rg)) = k::azure_state(self.frame) {
            let mut b = Vec::new();
            if let Some(rg) = rg {
                b.push(a("resource_group_name", q(rg)));
            }
            b.extend([
                a("storage_account_name", q(account)),
                a("container_name", q(container)),
                a("key", q(&format!("{}/{}.tfstate", self.source, self.name))),
                a("use_azuread_auth", "true"),
            ]);
            items.extend([Item::Blank, block("backend \"azurerm\"", b)]);
        }
        file(&self.header, vec![vec![block("terraform", items)]])
    }

    fn providers(&self) -> String {
        let ids = || vec![a("subscription_id", "var.subscription_id"), a("tenant_id", "var.tenant_id")];
        let mut rm = vec![block("features", vec![])];
        rm.extend(ids());
        rm.extend([
            Item::Blank,
            comment("Entra ID for data-plane calls: no storage account keys."),
            a("storage_use_azuread", "true"),
        ]);
        file(&self.header, vec![vec![block("provider \"azurerm\"", rm)], vec![block("provider \"azapi\"", ids())]])
    }

    fn main(&self) -> String {
        let mut groups: Vec<Vec<Item>> = Vec::new();
        let tags = obj(
            "tags",
            vec![
                a("\"rhizo:source\"", q(&self.source)),
                a("\"rhizo:environment\"", q(self.frame.id.as_str())),
                a("\"managed-by\"", q("opentofu")),
            ],
        );
        let mut locals = vec![
            a("location", q(&self.location)),
            comment("Globally unique names (storage, key vaults, apps) end in this: the same on every run."),
            a(
                "suffix",
                format!("substr(sha1(\"${{var.subscription_id}}/{}\"), 0, 6)", super::esc(&self.resource_group())),
            ),
        ];
        if self.service_bus() {
            locals.push(a("service_bus", format!("\"sbns-{}-${{local.suffix}}\"", cut(&self.name, 38))));
        }
        locals.extend([Item::Blank, tags]);
        groups.push(vec![block("locals", locals)]);
        if !self.of(KEY).collect::<Vec<_>>().is_empty() {
            groups.push(vec![block("data \"azurerm_client_config\" \"current\"", vec![])]);
        }
        groups.push(vec![
            comment(format!("rhizo:frame={}: everything in the environment lives here", self.frame.id)),
            {
                let body = vec![
                    a("name", q(&self.resource_group())),
                    a("location", "local.location"),
                    a("enable_telemetry", "false"),
                ];
                let mut items = vec![a("source", q(RESOURCE_GROUP.0)), a("version", q(RESOURCE_GROUP.1)), Item::Blank];
                items.extend(body);
                items.extend([Item::Blank, a("tags", "local.tags")]);
                block("module \"resource_group\"", items)
            },
        ]);
        if self.any(FUNCTION) {
            groups.extend(self.monitoring());
        }
        for n in &self.nodes {
            groups.extend(self.node_groups(n));
        }
        if self.service_bus() {
            groups.push(self.namespace());
        }
        if let Some(budget) = self.frame.props.get("budget-monthly") {
            groups.push(vec![
                comment(format!(
                    "rhizo:frame={} budget-monthly (the environment's resource group, in the billing currency)",
                    self.frame.id
                )),
                block(
                    "resource \"azurerm_consumption_budget_resource_group\" \"monthly\"",
                    vec![
                        a("name", q(&format!("{}-monthly", self.name))),
                        a("resource_group_id", "module.resource_group.resource_id"),
                        a("amount", budget.to_string()),
                        a("time_grain", q("Monthly")),
                        Item::Blank,
                        block(
                            "time_period",
                            vec![a("start_date", "formatdate(\"YYYY-MM-01'T'00:00:00Z\", plantimestamp())")],
                        ),
                        Item::Blank,
                        block(
                            "notification",
                            vec![
                                a("enabled", "true"),
                                a("operator", q("GreaterThan")),
                                a("threshold", "80"),
                                a("threshold_type", q("Forecasted")),
                                a("contact_emails", "var.budget_alert_emails"),
                            ],
                        ),
                        Item::Blank,
                        comment("The start is fixed by the first apply."),
                        block("lifecycle", vec![a("ignore_changes", "[time_period]")]),
                    ],
                ),
            ]);
        }
        file(&self.header, groups)
    }

    /// The code of one node in `main.tf`.
    pub(super) fn node_groups(&self, n: &Node) -> Vec<Vec<Item>> {
        let mut groups = Vec::new();
        let id = n.id.as_str();
        let node = |body: Vec<Item>, module: (&str, &str)| {
            vec![comment(format!("rhizo:node={id}")), module_block(&addr(id), module, body, id)]
        };
        match k::ty(n) {
            VPC => {
                if let Some(nat) = self.nat(n) {
                    groups.push(nat);
                }
                groups.push(node(self.vnet(n), VNET));
                if self.tiers(n).iter().any(|(t, _)| !self.dbs(n, t).is_empty()) {
                    groups.push(self.postgres_dns(n));
                }
            }
            BUCKET => groups.push(node(self.bucket(n), STORAGE)),
            TABLE => groups.push(node(self.table(n), COSMOS)),
            POSTGRES => groups.push(node(self.postgres(n), POSTGRES_MODULE)),
            KEY => {
                groups.push(node(self.key(n), KEY_VAULT));
                if self.used_keys().iter().any(|x| x.id == n.id) {
                    groups.push(self.identity(n, "the identity its stores reach the key with"));
                }
            }
            FUNCTION => groups.extend(self.function(n)),
            HTTP_API => groups.extend(self.api(n)),
            k::RESOURCE | k::MODULE => groups.push(generic(n)),
            _ => {} // subnets are tiers of their network; queues and topics live in one namespace
        }
        groups
    }

    /// One Log Analytics workspace and Application Insights per environment
    /// with functions: Azure Functions log nowhere else.
    fn monitoring(&self) -> Vec<Vec<Item>> {
        let ws = vec![
            a("source", q(WORKSPACE.0)),
            a("version", q(WORKSPACE.1)),
            Item::Blank,
            a("name", q(&format!("log-{}", self.name))),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("log_analytics_workspace_sku", q("PerGB2018")),
            a("log_analytics_workspace_retention_in_days", "30"),
            comment("Entra ID only; reachable without a private link scope."),
            a("log_analytics_workspace_local_authentication_enabled", "false"),
            a("log_analytics_workspace_internet_ingestion_enabled", q("true")),
            a("log_analytics_workspace_internet_query_enabled", q("true")),
            Item::Blank,
            a("tags", "local.tags"),
        ];
        let ai = vec![
            a("source", q(INSIGHTS.0)),
            a("version", q(INSIGHTS.1)),
            Item::Blank,
            a("name", q(&format!("appi-{}", self.name))),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("workspace_id", "module.monitoring_workspace.resource_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("retention_in_days", "30"),
            comment("Functions send telemetry with their managed identity (Monitoring Metrics Publisher)."),
            a("local_authentication_disabled", "true"),
            Item::Blank,
            a("tags", "local.tags"),
        ];
        vec![
            vec![
                comment(format!("rhizo:frame={}: logs and traces of its functions", self.frame.id)),
                block("module \"monitoring_workspace\"", ws),
            ],
            vec![block("module \"monitoring\"", ai)],
        ]
    }

    /// A user-assigned managed identity for `n`: `module.<n>_identity`.
    fn identity(&self, n: &Node, what: &str) -> Vec<Item> {
        let id = format!("{}_identity", addr(n.id.as_str()));
        vec![
            comment(format!("rhizo:node={}: {what}", n.id)),
            module_block(
                &id,
                IDENTITY,
                vec![
                    a("name", q(&format!("id-{}", self.base(n)))),
                    a("location", "local.location"),
                    a("resource_group_name", "module.resource_group.name"),
                    a("enable_telemetry", "false"),
                ],
                n.id.as_str(),
            ),
        ]
    }

    // --- Network ---------------------------------------------------------

    /// Subnet tiers of a network with their address prefixes: public and
    /// private always, database when a database is placed there or the tier
    /// is drawn. Azure subnets span every zone, so each tier is one subnet
    /// (the first zone's range of the AWS layout).
    fn tiers(&self, net: &Node) -> Vec<(&'static str, String)> {
        let cidr = k::text(net, "cidr").unwrap_or("10.0.0.0/16");
        let (base, len) = k::parse_cidr(cidr).unwrap_or((0x0a00_0000, 16));
        let subnet = |bits: u32, index: u32| {
            let ip = std::net::Ipv4Addr::from(base | (index << (32 - len - bits)));
            format!("{ip}/{}", len + bits)
        };
        let drawn = |t: &str| {
            net.children
                .iter()
                .filter_map(|c| self.doc.nodes.get(c))
                .any(|c| k::ty(c) == SUBNET && k::text(c, "tier") == Some(t))
        };
        let mut out = vec![("public", subnet(8, 48)), ("private", subnet(4, 0))];
        if drawn("database") || !self.tenants(net, "database").is_empty() {
            out.push(("database", subnet(8, 64)));
        }
        out
    }

    fn prefix(&self, net: &Node, tier: &str) -> String {
        self.tiers(net).into_iter().find(|(t, _)| *t == tier).map(|(_, p)| p).unwrap_or_default()
    }

    fn dbs(&self, net: &Node, tier: &str) -> Vec<&'a Node> {
        self.tenants(net, tier).into_iter().filter(|n| k::ty(n) == POSTGRES).collect()
    }

    fn nat(&self, net: &Node) -> Option<Vec<Item>> {
        let nat = k::text(net, "nat").unwrap_or("single");
        if nat == "none" {
            return None;
        }
        Some(vec![
            comment(format!(
                "rhizo:node={} nat={nat}: one zone-redundant StandardV2 NAT gateway serves every zone",
                net.id
            )),
            module_block(
                &format!("{}_nat", addr(net.id.as_str())),
                NAT,
                vec![
                    a("name", q(&format!("ng-{}", self.base(net)))),
                    a("location", "local.location"),
                    a("parent_id", "module.resource_group.resource_id"),
                    a("enable_telemetry", "false"),
                    a("public_ips", format!("{{ egress = {{ name = {} }} }}", q(&format!("pip-{}", self.base(net))))),
                ],
                net.id.as_str(),
            ),
        ])
    }

    fn vnet(&self, net: &Node) -> Vec<Item> {
        let cidr = k::text(net, "cidr").unwrap_or("10.0.0.0/16");
        let mut subnets = Vec::new();
        for (tier, prefix) in self.tiers(net) {
            let mut s = vec![a("name", q(&format!("snet-{tier}"))), a("address_prefixes", list([q(&prefix)]))];
            if tier == "private" && k::text(net, "nat").unwrap_or("single") != "none" {
                s.push(a("nat_gateway", format!("{{ id = module.{}_nat.resource_id }}", addr(net.id.as_str()))));
            }
            let tenants = self.tenants(net, tier);
            let service = if tenants.iter().any(|n| k::ty(n) == POSTGRES) {
                Some(("postgres", "Microsoft.DBforPostgreSQL/flexibleServers"))
            } else if tenants.iter().any(|n| k::ty(n) == FUNCTION) {
                Some(("functions", "Microsoft.App/environments"))
            } else {
                None
            };
            if !self.dbs(net, tier).is_empty() {
                s.push(a(
                    "network_security_group",
                    format!("{{ id = module.{}_{tier}_nsg.resource_id }}", addr(net.id.as_str())),
                ));
            }
            if let Some((what, svc)) = service {
                s.push(Item::Attr(
                    "delegations".into(),
                    Val::Objs(vec![vec![
                        a("name", q(what)),
                        a("service_delegation", format!("{{ name = {} }}", q(svc))),
                    ]]),
                ));
            }
            subnets.push(obj(tier, s));
        }
        let mut items = vec![
            a("name", q(&format!("vnet-{}", self.base(net)))),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("address_space", list([q(cidr)])),
            a("enable_telemetry", "false"),
            Item::Blank,
            comment("Subnets have no default outbound access: the private tier leaves through NAT."),
            obj("subnets", subnets),
        ];
        if k::flag(net, "flow-logs", true) {
            items.insert(5, comment("flow-logs: not compiled for azure (finding infra.azure.flow-logs)."));
        }
        items
    }

    fn postgres_dns(&self, net: &Node) -> Vec<Item> {
        let a_ = addr(net.id.as_str());
        vec![
            comment(format!("rhizo:node={}: names of the databases placed in it", net.id)),
            module_block(
                &format!("{a_}_postgres_dns"),
                DNS,
                vec![
                    a("domain_name", q(&format!("{}.private.postgres.database.azure.com", self.base(net)))),
                    a("parent_id", "module.resource_group.resource_id"),
                    a("enable_telemetry", "false"),
                    a(
                        "virtual_network_links",
                        format!("{{ {a_} = {{ name = {}, virtual_network_id = {}.resource_id }} }}", q(&a_), m(net)),
                    ),
                ],
                net.id.as_str(),
            ),
        ]
    }

    /// Network security groups for subnets that hold databases: Postgres
    /// from the subnets of functions with an edge to one, and from inside
    /// the subnet (high availability); nothing else from the network.
    fn network(&self) -> (Vec<Vec<Item>>, Vec<String>) {
        let mut groups = Vec::new();
        let mut nodes = BTreeSet::new();
        for net in self.of(VPC) {
            for (tier, prefix) in self.tiers(net) {
                let dbs = self.dbs(net, tier);
                if dbs.is_empty() {
                    continue;
                }
                nodes.insert(net.id.to_string());
                let mut rules = Vec::new();
                let mut sources: Vec<(String, Vec<String>)> = Vec::new();
                for (_, f, db, l) in self.links() {
                    if l != Link::Grant || !dbs.iter().any(|d| d.id == db.id) {
                        continue;
                    }
                    let Some(p) = k::placement(self.doc, f).filter(|p| p.vpc.id == net.id) else { continue };
                    nodes.extend([f.id.to_string(), db.id.to_string()]);
                    let edge = format!("{}->{}", f.id, db.id);
                    match sources.iter_mut().find(|(t, _)| t == p.tier) {
                        Some((_, es)) => es.push(edge),
                        None => sources.push((p.tier.to_string(), vec![edge])),
                    }
                }
                let rule = |key: &str, name: &str, access: &str, priority: u32, from: &str, port: &str, proto: &str| {
                    obj(
                        key,
                        vec![
                            a("name", q(name)),
                            a("access", q(access)),
                            a("direction", q("Inbound")),
                            a("priority", priority.to_string()),
                            a("protocol", q(proto)),
                            a("source_address_prefix", q(from)),
                            a("source_port_range", q("*")),
                            a("destination_address_prefix", q(&prefix)),
                            a("destination_port_range", q(port)),
                        ],
                    )
                };
                for (i, (t, edges)) in sources.iter().enumerate() {
                    rules.push(comment(format!("rhizo:edge={}", edges.join(" "))));
                    rules.push(rule(
                        &format!("postgres_from_{t}"),
                        &format!("allow-postgres-from-{t}"),
                        "Allow",
                        100 + i as u32,
                        &self.prefix(net, t),
                        "5432",
                        "Tcp",
                    ));
                }
                rules.push(rule("within", "allow-within-subnet", "Allow", 200, &prefix, "*", "*"));
                rules.push(rule("deny_network", "deny-network-inbound", "Deny", 4096, "VirtualNetwork", "*", "*"));
                groups.push(vec![
                    comment(format!(
                        "rhizo:node={} tier={tier}: {}",
                        net.id,
                        dbs.iter().map(|d| d.id.as_str()).collect::<Vec<_>>().join(" ")
                    )),
                    module_block(
                        &format!("{}_{tier}_nsg", addr(net.id.as_str())),
                        NSG,
                        vec![
                            a("name", q(&format!("nsg-{}-{tier}", self.base(net)))),
                            a("location", "local.location"),
                            a("parent_id", "module.resource_group.resource_id"),
                            a("enable_telemetry", "false"),
                            Item::Blank,
                            obj("security_rules", rules),
                        ],
                        net.id.as_str(),
                    ),
                ]);
            }
        }
        (groups, nodes.into_iter().collect())
    }

    // --- Stores -------------------------------------------------------------

    /// Customer-managed key inputs of a store encrypted with a key, shaped
    /// for the storage, Cosmos DB and Service Bus modules.
    fn cmk(&self, key: &Node) -> Vec<Item> {
        let km = m(key);
        vec![
            Item::Blank,
            comment(format!("rhizo:edge=…->{} (customer-managed key)", key.id)),
            a("managed_identities", format!("{{ user_assigned_resource_ids = [{km}_identity.resource_id] }}")),
            obj(
                "customer_managed_key",
                vec![
                    a("key_vault_resource_id", format!("{km}.resource_id")),
                    a("key_name", format!("{km}.keys[\"key\"].name")),
                    a("user_assigned_identity", format!("{{ resource_id = {km}_identity.resource_id }}")),
                ],
            ),
            a("depends_on", format!("[azurerm_role_assignment.{}_encryption]", addr(key.id.as_str()))),
        ]
    }

    /// Storage baseline shared by buckets and function host storage.
    fn storage_baseline(&self) -> Vec<Item> {
        vec![
            comment(
                "Baseline: Entra ID only (no shared keys), TLS 1.2, no public blobs; reachable from any network, like S3.",
            ),
            a("shared_access_key_enabled", "false"),
            a("default_to_oauth_authentication", "true"),
            a("allow_nested_items_to_be_public", "false"),
            a("https_traffic_only_enabled", "true"),
            a("min_tls_version", q("TLS1_2")),
            a("public_network_access_enabled", "true"),
            a("network_rules", "{ default_action = \"Allow\", bypass = [\"AzureServices\"] }"),
        ]
    }

    fn bucket(&self, n: &Node) -> Vec<Item> {
        let versioning = az::flag(n, "versioning", true);
        let mut items = vec![
            a("name", self.storage_name(n, "")),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("account_kind", q("StorageV2")),
            a("account_tier", q("Standard")),
            a("account_replication_type", q("ZRS")),
            a("access_tier", q(az::text(n, "access-tier").unwrap_or("Hot"))),
            Item::Blank,
        ];
        items.extend(self.storage_baseline());
        items.extend([
            Item::Blank,
            a("containers", format!("{{ {CONTAINER} = {{ name = {}, public_access = \"None\" }} }}", q(CONTAINER))),
            obj(
                "blob_properties",
                vec![
                    a("versioning_enabled", versioning.to_string()),
                    a("delete_retention_policy", "{ enabled = true, days = 7 }"),
                    a("container_delete_retention_policy", "{ enabled = true, days = 7 }"),
                ],
            ),
        ]);
        let rules: Vec<Item> = n
            .props
            .get("lifecycle")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, r)| {
                let prefix = r.get("prefix").and_then(Value::as_str).unwrap_or("");
                let days = r.get("expire-after-days").map(Value::to_string).unwrap_or_else(|| "1".into());
                let id =
                    if prefix.is_empty() { format!("expire-{}", i + 1) } else { format!("expire-{}", name(prefix)) };
                let mut actions =
                    vec![a("base_blob", format!("{{ delete_after_days_since_modification_greater_than = {days} }}"))];
                if versioning {
                    actions.push(a("version", format!("{{ delete_after_days_since_creation = {days} }}")));
                }
                obj(
                    &id.replace('-', "_"),
                    vec![
                        a("name", q(&id)),
                        a("enabled", "true"),
                        obj("actions", actions),
                        a(
                            "filters",
                            format!(
                                "{{ blob_types = [\"blockBlob\"], prefix_match = [{}] }}",
                                q(&format!("{CONTAINER}/{prefix}"))
                            ),
                        ),
                    ],
                )
            })
            .collect();
        if !rules.is_empty() {
            items.extend([Item::Blank, obj("storage_management_policy_rule", rules)]);
        }
        if let Some(key) = k::key(self.doc, n) {
            items.extend(self.cmk(key));
        }
        items
    }

    fn table(&self, n: &Node) -> Vec<Item> {
        let a_ = addr(n.id.as_str());
        let pk = az::text(n, "hash-key").unwrap_or("id");
        let mut container =
            vec![a("name", q(&name(n.id.as_str()))), a("partition_key_paths", list([q(&format!("/{pk}"))]))];
        if az::text(n, "ttl-attribute").is_some() {
            container.push(comment("ttl-attribute: items expire by their own `ttl` field (finding infra.azure.ttl)."));
            container.push(a("default_ttl", "-1"));
        }
        let pitr = az::flag(n, "point-in-time-recovery", true);
        let mut items = vec![
            a("name", self.global("cosmos", n, 44)),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("enable_telemetry", "false"),
            Item::Blank,
            comment("Serverless: billed per request, like on-demand capacity."),
            a("capabilities", "[{ name = \"EnableServerless\" }]"),
            a("consistency_policy", "{ consistency_level = \"Session\" }"),
            a(
                "backup",
                if pitr { "{ type = \"Continuous\", tier = \"Continuous30Days\" }" } else { "{ type = \"Periodic\" }" },
            ),
            Item::Blank,
            comment("Baseline: Entra ID only (no account keys), TLS 1.2."),
            a("local_authentication_disabled", "true"),
            a("minimal_tls_version", q("Tls12")),
            a("public_network_access_enabled", "true"),
            Item::Blank,
            obj(
                "sql_databases",
                vec![obj(&a_, vec![a("name", q(&name(n.id.as_str()))), obj("containers", vec![obj(&a_, container)])])],
            ),
        ];
        if az::flag(n, "deletion-protection", true) {
            items.extend([Item::Blank, a("lock", "{ kind = \"CanNotDelete\" }")]);
        }
        if let Some(key) = k::key(self.doc, n) {
            items.extend(self.cmk(key));
        }
        items
    }

    fn postgres(&self, n: &Node) -> Vec<Item> {
        let version = az::text(n, "engine-version").unwrap_or("17");
        let major = version.split('.').next().unwrap_or(version);
        let db = az::text(n, "database").map_or_else(|| addr(n.id.as_str()), str::to_string);
        let gb = az::int(n, "storage-gb", 20);
        let mb = PG_STORAGE_MB.into_iter().find(|mb| *mb >= gb * 1024).unwrap_or(gb * 1024);
        let a_ = addr(n.id.as_str());
        let mut items = vec![
            a("name", self.global("psql", n, 63)),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("server_version", q(major)),
            a("sku_name", q(az::text(n, "sku-name").unwrap_or(DEFAULT_SKU))),
        ];
        if mb != gb * 1024 {
            items.push(comment(format!("storage-gb={gb}, rounded up to a size Azure offers.")));
        }
        items.push(a("storage_mb", mb.to_string()));
        items.push(a(
            "high_availability",
            if az::flag(n, "multi-az", true) { "{ mode = \"ZoneRedundant\" }" } else { "null" },
        ));
        items.extend([
            a("backup_retention_days", az::int(n, "backup-days", 7).to_string()),
            a("databases", format!("{{ {a_} = {{ name = {} }} }}", q(&db))),
            Item::Blank,
            comment("Baseline: private (a delegated subnet), Entra ID only, no firewall rules."),
        ]);
        if let Some(p) = k::placement(self.doc, n) {
            items.extend([
                a("delegated_subnet_id", format!("{}.subnets[\"{}\"].resource_id", m(p.vpc), p.tier)),
                a("private_dns_zone_id", format!("module.{}_postgres_dns.resource_id", addr(p.vpc.id.as_str()))),
            ]);
        }
        items.extend([
            a("public_network_access_enabled", "false"),
            a("firewall_rules", "{}"),
            obj(
                "authentication",
                vec![
                    a("active_directory_auth_enabled", "true"),
                    a("password_auth_enabled", "false"),
                    a("tenant_id", "var.tenant_id"),
                ],
            ),
            obj(
                "ad_administrator",
                vec![obj(
                    "admin",
                    vec![
                        a("tenant_id", "var.tenant_id"),
                        a("object_id", format!("var.{a_}_admin_object_id")),
                        a("principal_name", format!("var.{a_}_admin_name")),
                        a("principal_type", q("Group")),
                    ],
                )],
            ),
        ]);
        if az::flag(n, "deletion-protection", true) {
            items.extend([Item::Blank, a("lock", "{ kind = \"CanNotDelete\" }")]);
        }
        if let Some(key) = k::key(self.doc, n) {
            let km = m(key);
            items.extend([
                Item::Blank,
                comment(format!("rhizo:edge={}->{} (customer-managed key)", n.id, key.id)),
                a("managed_identities", format!("{{ user_assigned_resource_ids = [{km}_identity.resource_id] }}")),
                obj(
                    "customer_managed_key",
                    vec![
                        a("key_vault_key_id", format!("{km}.keys[\"key\"].versionless_id")),
                        a("primary_user_assigned_identity_id", format!("{km}_identity.resource_id")),
                    ],
                ),
                a("depends_on", format!("[azurerm_role_assignment.{}_encryption]", addr(key.id.as_str()))),
            ]);
        }
        items
    }

    /// The environment's Service Bus namespace: every queue and topic.
    fn namespace(&self) -> Vec<Item> {
        let mut queues = Vec::new();
        for n in self.of(QUEUE) {
            let mut q_ = vec![a("name", q(&name(n.id.as_str())))];
            if az::flag(n, "fifo", false) {
                q_.push(a("requires_session", "true"));
            }
            q_.extend([
                a("default_message_ttl", q(&format!("PT{}S", az::int(n, "retention-s", 345_600)))),
                a("lock_duration", q(&format!("PT{}S", az::int(n, "visibility-timeout-s", 30)))),
                a("max_delivery_count", az::int(n, "max-receives", 5).to_string()),
                a("dead_lettering_on_message_expiration", az::flag(n, "dlq", true).to_string()),
            ]);
            queues.push(obj(&addr(n.id.as_str()), q_));
        }
        let mut topics = Vec::new();
        for t in self.of(TOPIC) {
            let ordered = az::flag(t, "fifo", false);
            let mut items = vec![a("name", q(&name(t.id.as_str())))];
            if ordered {
                items.push(a("support_ordering", "true"));
            }
            let subs: Vec<Item> = self
                .links_from(t, Link::Subscribe)
                .iter()
                .map(|(_, s)| {
                    let mut sub = vec![
                        comment(format!("rhizo:edge={}->{}", t.id, s.id)),
                        a("name", q(&name(s.id.as_str()))),
                        a("dead_lettering_on_message_expiration", "true"),
                    ];
                    if ordered {
                        sub.push(a("requires_session", "true"));
                    }
                    if k::ty(s) == QUEUE {
                        sub.push(a("forward_to", q(&name(s.id.as_str()))));
                    }
                    obj(&addr(s.id.as_str()), sub)
                })
                .collect();
            if !subs.is_empty() {
                items.push(obj("subscriptions", subs));
            }
            topics.push(obj(&addr(t.id.as_str()), items));
        }
        let key = az::service_bus_key(self.doc, self.frame).ok().flatten();
        let ids: Vec<&str> = self.of(QUEUE).chain(self.of(TOPIC)).map(|n| n.id.as_str()).collect();
        let mut items = vec![
            a("name", "local.service_bus"),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("enable_telemetry", "false"),
            Item::Blank,
        ];
        if key.is_some() {
            items.extend([
                comment("Premium: the only tier with customer-managed keys."),
                a("sku", q("Premium")),
                a("capacity", "1"),
                a("premium_messaging_partitions", "1"),
            ]);
        } else {
            items.push(a("sku", q("Standard")));
        }
        items.extend([
            Item::Blank,
            comment("Baseline: Entra ID only (no shared access keys), TLS 1.2."),
            a("local_auth_enabled", "false"),
            a("minimum_tls_version", q("1.2")),
            a("public_network_access_enabled", "true"),
        ]);
        if !queues.is_empty() {
            items.extend([Item::Blank, obj("queues", queues)]);
        }
        if !topics.is_empty() {
            items.extend([Item::Blank, obj("topics", topics)]);
        }
        if let Some(key) = key {
            items.extend(self.cmk(key));
        }
        vec![
            comment(format!("rhizo:node={}", ids.join(" "))),
            module_block("service_bus", SERVICE_BUS, items, &ids.join(" ")),
        ]
    }

    fn key(&self, n: &Node) -> Vec<Item> {
        let mut key = vec![
            a("name", q(&name(n.id.as_str()))),
            a("key_type", q("RSA-HSM")),
            a("key_size", "3072"),
            a("key_opts", list([q("unwrapKey"), q("wrapKey")])),
            obj(
                "rotation_policy",
                vec![
                    a("automatic", "{ time_after_creation = \"P365D\" }"),
                    a("expire_after", q("P730D")),
                    a("notify_before_expiry", q("P30D")),
                ],
            ),
        ];
        if let Some(d) = az::text(n, "description") {
            key.push(a("tags", format!("{{ description = {} }}", q(d))));
        }
        vec![
            a("name", self.global("kv", n, 24)),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("tenant_id", "var.tenant_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
            comment(
                "Baseline: HSM-backed keys, RBAC only, purge protection; reachable from any network with Entra ID.",
            ),
            a("sku_name", q("premium")),
            a("legacy_access_policies_enabled", "false"),
            a("purge_protection_enabled", "true"),
            a("soft_delete_retention_days", az::int(n, "deletion-window-days", 30).to_string()),
            a("public_network_access_enabled", "true"),
            a("network_acls", "{ default_action = \"Allow\", bypass = \"AzureServices\" }"),
            Item::Blank,
            obj("keys", vec![obj("key", key)]),
            Item::Blank,
            comment("Lets the identity that runs tofu create and rotate the key."),
            obj(
                "role_assignments",
                vec![obj(
                    "deployer",
                    vec![
                        a("role_definition_id_or_name", q("Key Vault Crypto Officer")),
                        a("principal_id", "data.azurerm_client_config.current.object_id"),
                    ],
                )],
            ),
        ]
    }

    // --- Compute ------------------------------------------------------------

    /// App settings a function finds its resources by, from its edges.
    pub(super) fn wiring(&self, n: &Node) -> Vec<(String, String)> {
        let mut vars = Vec::new();
        let mut bus = false;
        for (_, s) in self.links_from(n, Link::Grant) {
            let (ms, up, nm) = (m(s), upper(s.id.as_str()), name(s.id.as_str()));
            match k::ty(s) {
                BUCKET => vars.extend([
                    (format!("{up}_BLOB_ENDPOINT"), format!("\"https://${{{ms}.name}}.blob.core.windows.net\"")),
                    (format!("{up}_CONTAINER"), q(CONTAINER)),
                ]),
                TABLE => vars.extend([
                    (format!("{up}_COSMOS_ENDPOINT"), format!("{ms}.endpoint")),
                    (format!("{up}_DATABASE"), q(&nm)),
                    (format!("{up}_CONTAINER"), q(&nm)),
                ]),
                QUEUE => {
                    bus = true;
                    vars.push((format!("{up}_QUEUE"), q(&nm)));
                }
                TOPIC => {
                    bus = true;
                    vars.push((format!("{up}_TOPIC"), q(&nm)));
                }
                POSTGRES => vars.extend([
                    (format!("{up}_HOST"), format!("{ms}.fqdn")),
                    (format!("{up}_PORT"), q("5432")),
                    (
                        format!("{up}_NAME"),
                        q(&az::text(s, "database").map_or_else(|| addr(s.id.as_str()), str::to_string)),
                    ),
                    (format!("{up}_USER"), q(&format!("id-{}", self.base(n)))),
                ]),
                _ => {}
            }
        }
        for (_, src) in self.links_to(n, Link::Trigger) {
            bus = true;
            vars.push((format!("{}_QUEUE", upper(src.id.as_str())), q(&name(src.id.as_str()))));
        }
        for (_, t) in self.links_to(n, Link::Subscribe) {
            bus = true;
            vars.extend([
                (format!("{}_TOPIC", upper(t.id.as_str())), q(&name(t.id.as_str()))),
                (format!("{}_SUBSCRIPTION", upper(t.id.as_str())), q(&name(n.id.as_str()))),
            ]);
        }
        if bus {
            let identity = format!("module.{}_identity", addr(n.id.as_str()));
            vars.extend([
                (
                    format!("{SB_CONNECTION}__fullyQualifiedNamespace"),
                    "\"${local.service_bus}.servicebus.windows.net\"".into(),
                ),
                (format!("{SB_CONNECTION}__credential"), q("managedidentity")),
                (format!("{SB_CONNECTION}__clientId"), format!("{identity}.client_id")),
            ]);
        }
        let mut seen = BTreeSet::new();
        vars.retain(|(k_, _)| seen.insert(k_.clone()));
        vars
    }

    fn function(&self, n: &Node) -> Vec<Vec<Item>> {
        let a_ = addr(n.id.as_str());
        let id = n.id.as_str();
        let identity = format!("module.{a_}_identity");
        let storage = format!("module.{a_}_storage");
        let runtime = az::text(n, "runtime").unwrap_or("nodejs22.x");
        let (rt, version) = az::flex_runtime(runtime).unwrap_or(("node", "22".into()));
        let memory = az::flex_memory(n).unwrap_or(2048);
        let mut host = vec![
            a("name", self.storage_name(n, "fn")),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
        ];
        host.extend(self.storage_baseline());
        host.extend([
            Item::Blank,
            a("containers", "{ deployments = { name = \"deployments\", public_access = \"None\" } }"),
        ]);
        let plan = vec![
            a("name", q(&format!("asp-{}", self.base(n)))),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
            comment("Flex Consumption; zone redundancy is region-restricted, so it is off."),
            a("os_type", q("Linux")),
            a("sku_name", q("FC1")),
            a("zone_balancing_enabled", "false"),
        ];
        let mut settings = vec![
            ("AZURE_CLIENT_ID".to_string(), format!("{identity}.client_id")),
            (
                "APPLICATIONINSIGHTS_AUTHENTICATION_STRING".to_string(),
                format!("\"ClientId=${{{identity}.client_id}};Authorization=AAD\""),
            ),
        ];
        settings.extend(self.wiring(n));
        if let Some(user) = n.props.get("environment").and_then(Value::as_object) {
            for (key, v) in user {
                settings.retain(|(x, _)| x != key);
                settings.push((key.clone(), q(v.as_str().unwrap_or(""))));
            }
        }
        let mut site = vec![
            a("name", self.global("func", n, 60)),
            a("location", "local.location"),
            a("parent_id", "module.resource_group.resource_id"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("kind", q("functionapp")),
            a("os_type", q("Linux")),
            a("service_plan_resource_id", format!("module.{a_}_plan.resource_id")),
            a("function_app_uses_fc1", "true"),
            a("fc1_runtime_name", q(rt)),
            a("fc1_runtime_version", q(&version)),
        ];
        if n.props.get("azure").and_then(|b| b.get("instance-memory-mb")).is_none() {
            site.push(comment(format!(
                "memory-mb={}: the smallest Flex Consumption size that holds it.",
                az::int(n, "memory-mb", 256)
            )));
        }
        site.extend([
            a("instance_memory_in_mb", memory.to_string()),
            Item::Blank,
            comment(format!("The code is published to this app after apply (task T-{id}).")),
            a("managed_identities", format!("{{ user_assigned_resource_ids = [{identity}.resource_id] }}")),
            a("storage_account_name", format!("{storage}.name")),
            a("storage_uses_managed_identity", "true"),
            a("storage_user_assigned_identity_client_id", format!("{identity}.client_id")),
            a("storage_authentication_type", q("UserAssignedIdentity")),
            a("storage_user_assigned_identity_id", format!("{identity}.resource_id")),
            a(
                "storage_container_endpoint",
                format!("\"https://${{{storage}.name}}.blob.core.windows.net/deployments\""),
            ),
            a("application_insights_connection_string", "module.monitoring.connection_string"),
            Item::Blank,
            obj("app_settings", settings.into_iter().map(|(k_, v)| a(&k_, v)).collect()),
        ]);
        if let Some(p) = k::placement(self.doc, n) {
            site.extend([
                Item::Blank,
                a("virtual_network_subnet_id", format!("{}.subnets[\"{}\"].resource_id", m(p.vpc), p.tier)),
                a("vnet_route_all_traffic", "true"),
            ]);
        }
        site.extend([
            Item::Blank,
            comment("Baseline: HTTPS only, no basic-auth publishing; public for publishing and API Management."),
            a("https_only", "true"),
            a("public_network_access_enabled", "true"),
            a("ftp_publish_basic_authentication_enabled", "false"),
            a("scm_publish_basic_authentication_enabled", "false"),
            Item::Blank,
            comment("The app starts with its host storage and telemetry roles in place."),
            a(
                "depends_on",
                format!("[azurerm_role_assignment.{a_}_host_storage, azurerm_role_assignment.{a_}_telemetry]"),
            ),
        ]);
        vec![
            self.identity(n, "the identity the function runs as; its edges grant this identity"),
            vec![
                comment(format!("rhizo:node={id}: host storage and deployment packages")),
                module_block(&format!("{a_}_storage"), STORAGE, host, id),
            ],
            vec![comment(format!("rhizo:node={id}")), module_block(&format!("{a_}_plan"), PLAN, plan, id)],
            vec![comment(format!("rhizo:node={id}")), module_block(&a_, SITE, site, id)],
        ]
    }

    fn api(&self, n: &Node) -> Vec<Vec<Item>> {
        let a_ = addr(n.id.as_str());
        let id = n.id.as_str();
        let mut operations = Vec::new();
        let mut backends = Vec::new();
        let mut named = Vec::new();
        let mut keys = Vec::new();
        let mut methods: BTreeSet<String> = BTreeSet::new();
        for (e, f) in self.links_from(n, Link::Route) {
            let fa = addr(f.id.as_str());
            if !backends.iter().any(|(x, _): &(String, Item)| *x == fa) {
                let key = format!("{fa}-key");
                named.push(obj(
                    &fa,
                    vec![
                        a("display_name", q(&key)),
                        a("value", format!("random_password.{a_}_{fa}.result")),
                        a("secret", "true"),
                    ],
                ));
                backends.push((
                    fa.clone(),
                    obj(
                        &fa,
                        vec![
                            a("protocol", q("http")),
                            a("url", format!("\"https://${{module.{fa}.name}}.azurewebsites.net/api\"")),
                            a(
                                "credentials",
                                format!("{{ header = {{ \"x-functions-key\" = {} }} }}", q(&format!("{{{{{key}}}}}"))),
                            ),
                        ],
                    ),
                ));
                keys.push(f);
            }
            for r in k::routes(self.doc, e) {
                let (method, path) = match r.split_once(' ') {
                    Some((m_, p)) => (if m_ == "ANY" { "*" } else { m_ }, p.to_string()),
                    None => ("*", "/*".to_string()),
                };
                methods.insert(method.to_string());
                let params: Vec<String> = path
                    .split('/')
                    .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
                    .map(|p| format!("{{ name = {}, required = true, type = \"string\" }}", q(p)))
                    .collect();
                let slug = name(&path).replace('-', "_");
                let op_id = format!(
                    "{}_{}",
                    if method == "*" { "any" } else { method },
                    if slug.is_empty() { "root" } else { &slug }
                )
                .to_lowercase();
                let policy = format!(
                    "<policies><inbound><base /><set-backend-service backend-id=\"{fa}\" /></inbound><backend><base /></backend><outbound><base /></outbound><on-error><base /></on-error></policies>"
                );
                let mut op = vec![
                    comment(format!("rhizo:edge={}->{}", n.id, f.id)),
                    a("display_name", q(&r)),
                    a("method", q(method)),
                    a("url_template", q(&path)),
                ];
                if !params.is_empty() {
                    op.push(a("template_parameters", list(params)));
                }
                op.push(a("policy", format!("{{ xml_content = {} }}", q(&policy))));
                operations.push(obj(&op_id, op));
            }
        }
        let origins: Vec<&str> = n
            .props
            .get("cors-origins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut api = vec![
            a("display_name", q(&n.label)),
            comment("API Management needs a path: the routes are below it."),
            a("path", q(&name(n.id.as_str()))),
            a("protocols", "[\"https\"]"),
            comment("Open like the HTTP API it stands for: no subscription key."),
            a("subscription_required", "false"),
        ];
        if !origins.is_empty() {
            let methods: Vec<String> =
                if methods.contains("*") { vec!["*".into()] } else { methods.into_iter().collect() };
            let xml = format!(
                "<policies><inbound><cors allow-credentials=\"false\"><allowed-origins>{}</allowed-origins><allowed-methods preflight-result-max-age=\"3600\">{}</allowed-methods><allowed-headers><header>authorization</header><header>content-type</header></allowed-headers></cors><base /></inbound><backend><base /></backend><outbound><base /></outbound><on-error><base /></on-error></policies>",
                origins.iter().map(|o| format!("<origin>{o}</origin>")).collect::<String>(),
                methods.iter().map(|m_| format!("<method>{m_}</method>")).collect::<String>()
            );
            api.push(a("policy", format!("{{ xml_content = {} }}", q(&xml))));
        }
        if !operations.is_empty() {
            api.push(obj("operations", operations));
        }
        let mut items = vec![
            a("name", self.global("apim", n, 50)),
            a("location", "local.location"),
            a("resource_group_name", "module.resource_group.name"),
            a("enable_telemetry", "false"),
            Item::Blank,
            a("sku_name", q("Consumption_0")),
            a("publisher_name", q(&n.label)),
            a("publisher_email", format!("var.{a_}_publisher_email")),
        ];
        if !named.is_empty() {
            items.extend([
                Item::Blank,
                comment("Function keys API Management calls the functions with."),
                obj("named_values", named),
                obj("backends", backends.into_iter().map(|(_, b)| b).collect()),
            ]);
        }
        items.extend([Item::Blank, obj("apis", vec![obj(&a_, api)])]);
        let mut out = vec![vec![comment(format!("rhizo:node={id}")), module_block(&a_, APIM, items, id)]];
        for f in keys {
            let fa = addr(f.id.as_str());
            let key = format!("apim-{}", name(id));
            out.push(vec![
                comment(format!("rhizo:edge={}->{}: the function key API Management sends", n.id, f.id)),
                block(
                    format!("resource \"random_password\" \"{a_}_{fa}\""),
                    vec![a("length", "48"), a("special", "false")],
                ),
            ]);
            out.push(vec![block(
                format!("resource \"azapi_resource_action\" \"{a_}_{fa}\""),
                vec![
                    a("type", q("Microsoft.Web/sites/host@2024-04-01")),
                    a("resource_id", format!("\"${{module.{fa}.resource_id}}/host/default\"")),
                    a("action", q(&format!("functionKeys/{key}"))),
                    a("method", q("PUT")),
                    a(
                        "body",
                        format!(
                            "{{ properties = {{ name = {}, value = random_password.{a_}_{fa}.result }} }}",
                            q(&key)
                        ),
                    ),
                ],
            )]);
        }
        out
    }

    // --- Identity and access ---------------------------------------------

    /// Role assignments: each function's identity gets what its edges
    /// say, scoped to the one resource; stores get their keys.
    fn iam(&self) -> (Vec<Vec<Item>>, Vec<String>) {
        let mut groups = Vec::new();
        let mut nodes = BTreeSet::new();
        let mut custom: BTreeSet<&'static str> = BTreeSet::new();
        // `role` is a built-in role's name, or a custom role's `azurerm_role_definition`.
        let assign = |name: String,
                      edge: String,
                      scope: String,
                      role: &str,
                      principal: String,
                      condition: Option<String>| {
            let role = match role.strip_prefix("azurerm_role_definition.") {
                Some(r) => a("role_definition_id", format!("azurerm_role_definition.{r}.role_definition_resource_id")),
                None => a("role_definition_name", q(role)),
            };
            let mut items =
                vec![a("scope", scope), role, a("principal_id", principal), a("principal_type", q("ServicePrincipal"))];
            if let Some(c) = condition {
                items.extend([a("condition_version", q("2.0")), a("condition", q(&c))]);
            }
            vec![comment(edge), block(format!("resource \"azurerm_role_assignment\" \"{name}\""), items)]
        };
        for key in self.used_keys() {
            let ka = addr(key.id.as_str());
            nodes.insert(key.id.to_string());
            groups.push(assign(
                format!("{ka}_encryption"),
                format!("rhizo:node={}: stores encrypted with it use the key through this identity", key.id),
                format!("\"${{module.{ka}.resource_id}}/keys/${{module.{ka}.keys[\"key\"].name}}\""),
                KEY_USER,
                format!("module.{ka}_identity.principal_id"),
                None,
            ));
        }
        for f in self.of(FUNCTION) {
            let fa = addr(f.id.as_str());
            let principal = format!("module.{fa}_identity.principal_id");
            nodes.insert(f.id.to_string());
            groups.push(assign(
                format!("{fa}_host_storage"),
                format!("rhizo:node={}: its host storage and deployment packages", f.id),
                format!("module.{fa}_storage.resource_id"),
                BLOB_OWNER,
                principal.clone(),
                None,
            ));
            groups.push(assign(
                format!("{fa}_telemetry"),
                format!("rhizo:node={}: sends telemetry to Application Insights", f.id),
                "module.monitoring.resource_id".into(),
                METRICS_PUBLISHER,
                principal.clone(),
                None,
            ));
            for (e, s) in self.links_from(f, Link::Grant) {
                let Ok((read, write)) = k::access(e, s) else { continue };
                let access = match (read, write) {
                    (true, true) => "read-write",
                    (true, false) => "read",
                    _ => "write",
                };
                nodes.insert(s.id.to_string());
                let sa = addr(s.id.as_str());
                let trace = format!("rhizo:edge={}->{} access={access}", f.id, s.id);
                match k::ty(s) {
                    BUCKET => {
                        let prefix = e.props.get("prefix").and_then(Value::as_str).unwrap_or("");
                        let scope = format!("module.{sa}.containers[{}].id", q(CONTAINER));
                        let (role, actions): (String, Vec<&str>) = match access {
                            "read" => (BLOB_READER.into(), vec!["read"]),
                            "write" => {
                                custom.insert("write");
                                ("azurerm_role_definition.blob_writer".into(), vec!["write", "add/action"])
                            }
                            _ => {
                                custom.insert("read-write");
                                (
                                    "azurerm_role_definition.blob_reader_writer".into(),
                                    vec!["read", "write", "add/action"],
                                )
                            }
                        };
                        let condition = (!prefix.is_empty()).then(|| {
                            let not: Vec<String> =
                                actions.iter().map(|x| format!("!(ActionMatches{{'{BLOB_ACTIONS}/{x}'}})")).collect();
                            format!(
                                "(({}) OR (@Resource[{BLOB_ACTIONS}:path] StringStartsWith '{}'))",
                                not.join(" AND "),
                                prefix.replace('\'', "")
                            )
                        });
                        let trace = if prefix.is_empty() { trace } else { format!("{trace} prefix={prefix}") };
                        groups.push(assign(format!("{fa}_{sa}"), trace, scope, &role, principal.clone(), condition));
                    }
                    TABLE => {
                        let (role, what) = match access {
                            "read" => (
                                format!("\"${{module.{sa}.resource_id}}/sqlRoleDefinitions/{COSMOS_READER}\""),
                                "Cosmos DB Built-in Data Reader",
                            ),
                            "write" => (format!("azurerm_cosmosdb_sql_role_definition.{sa}_writer.id"), "writes only"),
                            _ => (
                                format!("\"${{module.{sa}.resource_id}}/sqlRoleDefinitions/{COSMOS_CONTRIBUTOR}\""),
                                "Cosmos DB Built-in Data Contributor",
                            ),
                        };
                        if access == "write" {
                            groups.push(vec![
                                comment(format!("rhizo:node={}: write without read, which no built-in role grants", s.id)),
                                block(
                                    format!("resource \"azurerm_cosmosdb_sql_role_definition\" \"{sa}_writer\""),
                                    vec![
                                        a("name", q(&format!("{}-writer", name(s.id.as_str())))),
                                        a("resource_group_name", "module.resource_group.name"),
                                        a("account_name", format!("module.{sa}.name")),
                                        a("type", q("CustomRole")),
                                        a("assignable_scopes", format!("[module.{sa}.resource_id]")),
                                        Item::Blank,
                                        block(
                                            "permissions",
                                            vec![a(
                                                "data_actions",
                                                list([
                                                    "Microsoft.DocumentDB/databaseAccounts/readMetadata",
                                                    "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/items/create",
                                                    "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/items/delete",
                                                    "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/items/replace",
                                                    "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/items/upsert",
                                                ]
                                                .map(q)),
                                            )],
                                        ),
                                    ],
                                ),
                            ]);
                        }
                        let nm = name(s.id.as_str());
                        groups.push(vec![
                            comment(format!("{trace} ({what})")),
                            block(
                                format!("resource \"azurerm_cosmosdb_sql_role_assignment\" \"{fa}_{sa}\""),
                                vec![
                                    a("resource_group_name", "module.resource_group.name"),
                                    a("account_name", format!("module.{sa}.name")),
                                    a("role_definition_id", role),
                                    a("principal_id", principal.clone()),
                                    a("scope", format!("\"${{module.{sa}.resource_id}}/dbs/{nm}/colls/{nm}\"")),
                                ],
                            ),
                        ]);
                    }
                    QUEUE | TOPIC => {
                        let scope = if k::ty(s) == QUEUE {
                            format!("module.service_bus.resource_queues[{}].id", q(&name(s.id.as_str())))
                        } else {
                            format!("module.service_bus.resource_topics[{}].id", q(&name(s.id.as_str())))
                        };
                        if write {
                            groups.push(assign(
                                format!("{fa}_{sa}_send"),
                                trace.clone(),
                                scope.clone(),
                                SB_SENDER,
                                principal.clone(),
                                None,
                            ));
                        }
                        if read {
                            groups.push(assign(
                                format!("{fa}_{sa}_receive"),
                                trace.clone(),
                                scope,
                                SB_RECEIVER,
                                principal.clone(),
                                None,
                            ));
                        }
                    }
                    POSTGRES => groups.push(vec![comment(format!(
                        "{trace}: Entra ID sign-in as id-{}; the database role is created in SQL (task T-{})",
                        self.base(f),
                        f.id
                    ))]),
                    _ => {}
                }
            }
            for (_, qn) in self.links_to(f, Link::Trigger) {
                nodes.insert(qn.id.to_string());
                let qa = addr(qn.id.as_str());
                groups.push(assign(
                    format!("{fa}_{qa}_trigger"),
                    format!("rhizo:edge={}->{} (trigger)", qn.id, f.id),
                    format!("module.service_bus.resource_queues[{}].id", q(&name(qn.id.as_str()))),
                    SB_RECEIVER,
                    principal.clone(),
                    None,
                ));
            }
            for (_, t) in self.links_to(f, Link::Subscribe) {
                nodes.insert(t.id.to_string());
                let ta = addr(t.id.as_str());
                groups.push(assign(
                    format!("{fa}_{ta}_subscription"),
                    format!("rhizo:edge={}->{} (subscription)", t.id, f.id),
                    format!(
                        "module.service_bus.resource_topics_subscriptions[\"{}|{}\"].id",
                        name(t.id.as_str()),
                        name(f.id.as_str())
                    ),
                    SB_RECEIVER,
                    principal.clone(),
                    None,
                ));
            }
        }
        let mut defs = Vec::new();
        for (which, res, actions) in [
            ("read-write", "blob_reader_writer", vec!["read", "write", "add/action"]),
            ("write", "blob_writer", vec!["write", "add/action"]),
        ] {
            if !custom.contains(which) {
                continue;
            }
            let title = if which == "write" { "Blob Data Writer" } else { "Blob Data Reader and Writer" };
            defs.push(vec![
                comment(format!("Blob access={which} without delete, which no built-in role grants.")),
                block(
                    format!("resource \"azurerm_role_definition\" \"{res}\""),
                    vec![
                        a("name", format!("\"{title} (${{module.resource_group.name}})\"")),
                        a("scope", "module.resource_group.resource_id"),
                        a("assignable_scopes", "[module.resource_group.resource_id]"),
                        Item::Blank,
                        block(
                            "permissions",
                            vec![a("data_actions", list(actions.iter().map(|x| q(&format!("{BLOB_ACTIONS}/{x}")))))],
                        ),
                    ],
                ),
            ]);
        }
        defs.extend(groups);
        (defs, nodes.into_iter().collect())
    }

    // --- Inputs and outputs --------------------------------------------------

    /// The root module's variables: each name with its block.
    pub(super) fn variable_blocks(&self) -> Vec<(String, Item)> {
        let var = |name: &str, ty: &str, desc: String, extra: Vec<Item>| {
            let mut items = vec![a("description", q(&desc)), a("type", ty)];
            if !extra.is_empty() {
                items.push(Item::Blank);
                items.extend(extra);
            }
            (name.to_string(), block(format!("variable \"{name}\""), items))
        };
        let uuid = |v: &str, what: &str| {
            vec![block(
                "validation",
                vec![
                    a(
                        "condition",
                        format!(
                            "can(regex(\"^[0-9a-f]{{8}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{4}}-[0-9a-f]{{12}}$\", var.{v}))"
                        ),
                    ),
                    a("error_message", q(&format!("{v} is {what}, a GUID."))),
                ],
            )]
        };
        let mut out = vec![
            var(
                "subscription_id",
                "string",
                format!("The Azure subscription the {} environment is deployed to.", self.frame.id),
                uuid("subscription_id", "an Azure subscription id"),
            ),
            var(
                "tenant_id",
                "string",
                "The Microsoft Entra ID tenant of the subscription.".into(),
                uuid("tenant_id", "a Microsoft Entra ID tenant id"),
            ),
        ];
        if self.frame.props.contains_key("budget-monthly") {
            out.push(var("budget_alert_emails", "list(string)", "Who gets budget alerts.".into(), vec![]));
        }
        for n in self.of(POSTGRES) {
            let a_ = addr(n.id.as_str());
            out.push(var(
                &format!("{a_}_admin_object_id"),
                "string",
                format!("Object id of the Entra ID group that administers {} ({}).", n.label, n.id),
                uuid(&format!("{a_}_admin_object_id"), "an Entra ID object id"),
            ));
            out.push(var(
                &format!("{a_}_admin_name"),
                "string",
                "Display name of that Entra ID group (the database role it signs in as).".to_string(),
                vec![],
            ));
        }
        for n in self.of(HTTP_API) {
            out.push(var(
                &format!("{}_publisher_email", addr(n.id.as_str())),
                "string",
                format!("Who API Management notifies about {} ({}).", n.label, n.id),
                vec![],
            ));
        }
        out
    }

    fn outputs(&self) -> String {
        let mut groups = vec![vec![block(
            "output \"resource_group\"",
            vec![
                a("description", q(&format!("Resource group of the {} environment", self.frame.id))),
                a("value", "module.resource_group.name"),
            ],
        )]];
        for n in &self.nodes {
            let ma = m(n);
            let a_ = addr(n.id.as_str());
            let (suffix, value, what) = match k::ty(n) {
                VPC => ("vnet_id", format!("{ma}.resource_id"), "Virtual network id"),
                BUCKET => ("storage_account", format!("{ma}.name"), "Storage account (blob container `data`)"),
                TABLE => ("endpoint", format!("{ma}.endpoint"), "Cosmos DB endpoint"),
                POSTGRES => ("fqdn", format!("{ma}.fqdn"), "Database host name"),
                QUEUE => (
                    "queue_id",
                    format!("module.service_bus.resource_queues[{}].id", q(&name(n.id.as_str()))),
                    "Service Bus queue id",
                ),
                TOPIC => (
                    "topic_id",
                    format!("module.service_bus.resource_topics[{}].id", q(&name(n.id.as_str()))),
                    "Service Bus topic id",
                ),
                KEY => ("key_id", format!("{ma}.keys_resource_ids[\"key\"].versionless_id"), "Key Vault key URI"),
                FUNCTION => ("function_name", format!("{ma}.name"), "Function App name (publish the code here)"),
                HTTP_API => ("url", format!("\"${{{ma}.apim_gateway_url}}/{}\"", name(n.id.as_str())), "Base URL"),
                _ => continue,
            };
            groups.push(vec![block(
                format!("output \"{a_}_{suffix}\""),
                vec![a("description", q(&format!("{what} of {} ({})", n.label, n.id))), a("value", value)],
            )]);
        }
        file(&self.header, groups)
    }
}

/// What the generated Azure infrastructure expects of a function's code.
pub(super) fn function_criteria(doc: &Document, frame: &Frame, f: &Node) -> Vec<Criterion> {
    let env = Az::new(doc, frame, "");
    let dir = dir(frame);
    let fa = addr(f.id.as_str());
    let runtime = az::text(f, "runtime").unwrap_or("nodejs22.x");
    let (rt, version) = az::flex_runtime(runtime).unwrap_or(("node", "22".into()));
    let timeout = az::int(f, "timeout-s", 10);
    let mut out = vec![
        Criterion::new(
            "infra",
            format!(
                "Runs on Flex Consumption runtime `{rt} {version}`; after `{dir}` is applied it is published to the Function App named by output `{fa}_function_name` (`func azure functionapp publish`)"
            ),
        ),
        Criterion::new(
            "infra",
            format!(
                "`host.json` sets `functionTimeout` to `{:02}:{:02}:{:02}` (timeout-s={timeout})",
                timeout / 3600,
                timeout / 60 % 60,
                timeout % 60
            ),
        ),
    ];
    let env_vars: Vec<String> = env.wiring(f).into_iter().map(|(k_, _)| format!("`{k_}`")).collect();
    out.push(Criterion::new(
        "infra",
        format!(
            "Signs in with its managed identity (`AZURE_CLIENT_ID`, e.g. DefaultAzureCredential), never keys or connection strings{}",
            if env_vars.is_empty() {
                String::new()
            } else {
                format!(", and finds its resources through the app settings in `{dir}/main.tf` ({})", env_vars.join(", "))
            }
        ),
    ));
    for (_, e, a_, b, l) in k::links(doc) {
        match l {
            Link::Grant if a_.id == f.id && k::ty(b) == POSTGRES => out.push(Criterion::new(
                "infra",
                format!(
                    "Connects to `{}` over TLS as the Entra ID principal `id-{}-{}` with a token from its managed identity (no password); a migration run by the database's Entra admin creates that role (`pgaadauth_create_principal`)",
                    b.id,
                    name(frame.id.as_str()),
                    name(f.id.as_str())
                ),
            )),
            Link::Trigger if b.id == f.id => out.push(Criterion::new(
                "infra",
                format!(
                    "Is triggered by Service Bus queue `%{}_QUEUE%` (connection `{SB_CONNECTION}`{}) and lets a failed message be retried and then dead-lettered",
                    upper(a_.id.as_str()),
                    if az::flag(a_, "fifo", false) { ", sessions on" } else { "" }
                ),
            )),
            Link::Subscribe if b.id == f.id => out.push(Criterion::new(
                "infra",
                format!(
                    "Handles the messages of topic `%{0}_TOPIC%` (subscription `%{0}_SUBSCRIPTION%`, connection `{SB_CONNECTION}`{1}) idempotently (Service Bus may deliver a message more than once)",
                    upper(a_.id.as_str()),
                    if az::flag(a_, "fifo", false) { ", sessions on" } else { "" }
                ),
            )),
            Link::Route if b.id == f.id => {
                for r in k::routes(doc, e) {
                    out.push(Criterion::new(
                        "infra",
                        format!(
                            "Serves `{r}` as an HTTP trigger with `authLevel` function under `/api`: API Management calls it with a function key"
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    if k::links(doc).any(|(_, _, a_, _, l)| a_.id == f.id && l == Link::Grant) {
        out.push(Criterion::new(
            "infra",
            format!("Needs no access beyond the role assignments in `{dir}/iam.tf`; a missing one is a change to the diagram"),
        ));
    }
    out
}
