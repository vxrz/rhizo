# Cloud-neutral infrastructure

Status: draft, 2026-10-22. Builds on `profiles-and-compiler.md` §3.7 (`infra` kit) and `iac-import.md`.

## Goal

Draw infrastructure once and compile it for AWS, Azure or Google Cloud. The diagram is cloud-neutral; the generated OpenTofu is idiomatic for one cloud and uses that cloud's maintained modules. Importing existing code works for any provider, and the round-trip check (L1) runs offline for every cloud.

What rhizo does **not** promise: one piece of code that runs on every cloud. Switching clouds means changing `cloud=` and recompiling. Anything that does not carry over is named (a warning on the node, an open question in the brief), never dropped silently.

## Where we are

| Part | Today | Cloud-specific because |
|---|---|---|
| Import (`rhizo import tofu`) | Any provider: every block keeps its HCL as text; recognized resources and modules of aws, azurerm and google fold into the neutral types (`iac-import.md`, phase 2) | the recognizers per cloud |
| Adopt (`rhizo adopt`) | Imported nodes switch to rhizo's code with `moved` blocks, props editable: rhizo's own module calls on aws, azure and gcp (and rhizo's own Google resources); raw S3 buckets with their satellites, SQS queues, SNS topics, KMS keys and DynamoDB tables on aws (`iac-import.md`, "Adopt (switching shapes) as built") | the shape tables per cloud; raw Azure and Google resources are not adopted yet |
| Generic nodes in designed environments | Any provider | nothing |
| L1 offline plan | aws, azurerm, google (since 2026-10-22, §1) | mock profiles exist for these three |
| Designed types | 10 `aws.*` types → terraform-aws-modules; 10 neutral `infra.*` types → terraform-aws-modules on aws, Azure Verified Modules on azure, terraform-google-modules on gcp (§2, §2a, §2b) | the kit's vocabulary |

## 1. L1 with mocked providers (built)

L1 plans the original and the compiled code and compares the plans. Instead of real providers with fake credentials, it runs `tofu test` with `mock_provider` for every provider. A generated test file holds one run block with `command = plan`. `tofu test -json -verbose` emits a `test_plan` event that has the same shape as `tofu show -json` (`planned_values`, `resource_changes`, `output_changes`, `configuration`), so the comparison code stays as it is.

Spike results (OpenTofu 1.13.2, aws 6.69.1, azurerm 4.81.0, google 6.50.0):

- **Offline.** No credentials, environment cleared, and outbound network denied by the macOS sandbox: plans still pass. Only `tofu init` needs the registry, as before.
- **Deterministic.** Mocked values do not depend on file layout or run: two runs, and the same code split differently across files, give identical plans apart from timestamps. Values depend on the address, so original and compiled code see the same values.
- **rhizo's own output now plans.** The `shop` OpenTofu (87 resources, pinned modules with their own `aws_caller_identity` and `aws_availability_zones` reads) plans in about 2.5 s. With fake credentials it failed on STS, the EC2 API and `allowed_account_ids`.
- **Azure and Google.** A storage account, container, key vault, role assignment, GCS bucket and IAM member plan offline.

Mocked values must be **valid**, not real. Providers still validate them, and random strings fail those checks. rhizo writes a **mock profile** per provider:

- **Generated from the provider schema** (`tofu providers schema -json`, about 1.3 s for aws): every attribute that is computed and not optional (and `id`, which no configuration can set) gets a valid default. For aws: `arn` and `*_arn` get an ARN, and `policy`, `json` and `*_policy` get an empty policy document. That is 1,347 blocks for aws 6.69.1. Attributes the configuration can set must not be mocked: OpenTofu refuses that.
- **Curated defaults** for the data sources code indexes into: `aws_availability_zones` (three zones, since an empty list breaks `names[0]` and `slice`), `aws_caller_identity`, `aws_region`, `aws_partition`; `azurerm_client_config` (UUIDs).
- **Azure resource IDs.** azurerm parses IDs by resource kind (a container needs a `…/Microsoft.Storage/storageAccounts/…` ID), so each `azurerm_*` resource needs its ID shape. Each resource page in the azurerm docs has an "Import" section with an example ID. The table is generated from those docs once per pinned provider version and shipped with the kit; types missing from it fail with a message naming the type.
- **Google**: a service account's `email` and `member`.
- **Other providers** get a plain `mock_provider` and grow a profile when a corpus entry needs one.

Consequences for L1:
- The fake-credential override is gone. OpenTofu's built-in `terraform` provider (`terraform_data`) stays real; `external` is mocked, so no program from the imported code runs. The local-backend override and cutting `import` and `encryption` blocks stay (`providers schema` still evaluates encryption).
- `variables` and `checks` are not in `test_plan`. Variable values are the same on both sides (the same tfvars), and the `configuration` comparison covers declarations and `check` blocks.
- Data sources return constants, so a change in a data source's arguments shows only in `configuration`, not in planned values. That is enough, because both sides are compared on both.
- `mock_provider` needs one block per provider alias; the generator reads aliases from the provider blocks.
- In `tofu test` plans, `planned_values.outputs` also holds every module's outputs under bare names, so values collide; L1 drops that map and compares `output_changes`.
- As built: `crates/iac/src/mocks.rs`; the Azure ID table `crates/iac/src/mocks/azurerm_ids.tsv` (1,129 types, from the azurerm v4.81.0 docs). Corpus: `azure-storage`, `gcp-storage`, and `rhizo_output_round_trips` (the `shop` output, 87 resources). A changed diagram is still reported by argument path.

## 2. Neutral types (built for aws, 2026-10-22)

The infra kit gains cloud-neutral types next to the `aws.*` ones:

| Type | AWS | Azure | Google Cloud |
|---|---|---|---|
| `infra.network` (container node, holds `infra.subnet` tiers) | VPC + subnets | VNet + subnets | VPC + subnetworks |
| `infra.bucket` | S3 | Storage account + blob container | Cloud Storage bucket |
| `infra.queue` | SQS | Service Bus queue | Pub/Sub subscription |
| `infra.topic` | SNS | Service Bus topic | Pub/Sub topic |
| `infra.function` | Lambda | Function App (Flex Consumption) | Cloud Run function |
| `infra.database` (`engine=postgres`) | RDS | Database for PostgreSQL Flexible Server | Cloud SQL |
| `infra.table` | DynamoDB | Cosmos DB (NoSQL) | Firestore |
| `infra.key` | KMS | Key Vault key | Cloud KMS |
| `infra.api` | API Gateway HTTP | API Management | API Gateway |
| modules | terraform-aws-modules | Azure Verified Modules | terraform-google-modules |

Every row needs its module checked and pinned in the catalog under the admission policy (Q9), as for AWS today. Where a cloud has no maintained module for a row, the compiler emits raw resources.

**Choosing the cloud.** `infra.environment` gets `cloud="aws" | "azure" | "gcp"`, next to `region`. A neutral node compiles for its environment's cloud. Two environments of one project can target different clouds.

**As built (step 2).**
- `infra.environment` has `cloud` (`aws`, `azure`, `gcp`; default `aws`), replacing the unused `provider` prop. The AWS region pattern applies only when the cloud is aws. A designed environment with a cloud rhizo could not compile yet was an error (`[infra.cloud.unsupported]`); since step 4 every value the schema allows compiles, so the rule is gone. Imported environments compile back verbatim whatever their cloud.
- The ten neutral types are in the infra kit with their own schemas, ports, parents (a neutral subnet may sit in an `aws.vpc` and vice versa) and looks. `rhizo_kit::infra::ty` answers a neutral node with its AWS type and `prop`/`text`/`int`/`flag` read renamed props and the `aws` block, so the rules, the compiler and the brief need no other change. Rule messages name the type the person wrote (`infra.subnet`, "network").
- Renamed props: `high-availability` (multi-az), `partition-key`/`partition-key-type`/`sort-key`/`sort-key-type` (hash/range key), `ordered` (fifo), `lock-timeout-s` (visibility timeout), `dead-letter` (dlq), `max-deliveries` (max receives), `zones` (azs). AWS-only, in the `aws` block: `instance-class` (database), `architecture` and `log-retention-days` (function). `infra.database` adds `engine` (`postgres`).
- Proof: the compiler's kitchen-sink environment rewritten with neutral types compiles to byte-identical `infra/` files and the same brief tasks; the `shop` example rewritten likewise compiles to identical OpenTofu; the byte-for-byte baseline of all examples is unchanged. The `infra` starter and the AI guide now use neutral types.
- Not yet: the neutral `size` fallback. Imported environments get `cloud` from their providers since step 5. The `azure` block and the warning for another cloud's block came with step 3 (§2a), the `gcp` block with step 4 (§2b).

**Props.**
- *Neutral props* cover what every cloud has: `versioning`, `retention`, `encrypted` (always on by default), `public` (always off by default), `runtime`, `memory-mb`, `timeout-s`, `engine-version`, `storage-gb`, `high-availability`.
- *Per-cloud props* go in a child block named after the cloud: `aws { object-lock true }`, `azure { access-tier "Cool" }`. They apply only when compiling for that cloud. For another cloud the node gets a warning, and the brief lists the setting as an open question ("`azure.access-tier` has no Google equivalent: drop it or choose a storage class").
- *Sizes* stay explicit per cloud (`aws { instance-class "db.t4g.small" }`), with a neutral `size="small" | "medium" | "large"` fallback mapped per cloud in the catalog.

**Edges stay neutral.** The edge rules of §3.7 already speak in intent:

| Edge | AWS | Azure | Google Cloud |
|---|---|---|---|
| `data access=read` to a bucket | IAM policy statement | role assignment "Storage Blob Data Reader" on the container | IAM binding `roles/storage.objectViewer` |
| `data access=write` to a queue | `sqs:SendMessage` | "Azure Service Bus Data Sender" | `roles/pubsub.publisher` |
| `data` to a database | security group ingress | private endpoint + NSG rule | private IP + firewall rule |
| `dependency` from compute to `infra.network` | Lambda in private subnets | VNet integration | Serverless VPC Access / Direct VPC egress |
| `control` from `infra.api` to `infra.function` | route + invoke permission | API + backend | gateway route + invoker binding |

Identity is per compute node: an IAM role on AWS, a user-assigned managed identity on Azure, a service account on Google. The edges grant to that identity.

**Capability matrix.** As for the `flow` targets: each cloud's compiler declares the types, props and edge kinds it supports. An unsupported combination is an error on the node that names the cloud, never a silent approximation.

**`aws.*` types stay.** They remain the precise vocabulary for AWS-only designs, and imported diagrams fold into them (`iac-import.md` phase 2). A command converts between `aws.s3.bucket` and `infra.bucket` when the props allow it. Today's AWS output must not change: the AWS compiler for neutral types reuses the `aws.*` emitters, and the byte-for-byte baseline of the examples must stay identical.

## 2a. The Azure compiler (built, step 3, 2026-10-22)

An environment with `cloud="azure"` compiles to `infra/<env>/` from `crates/compile/src/infra/azure.rs`; aws environments go through the unchanged AWS emitter, and the byte-for-byte baseline of every existing example is unchanged. Only neutral types compile on azure: an `aws.*` node there is an error naming it (`[infra.cloud.type]`); `infra.resource`/`infra.module` compile verbatim as everywhere.

**Files.** `versions.tf` (OpenTofu `>= 1.11.1, < 2.0.0`, five providers pinned exactly, an `azurerm` backend with `use_azuread_auth = true` when the environment's `azure` block names a state storage account), `providers.tf` (`subscription_id`/`tenant_id` from variables, `storage_use_azuread`), `main.tf`, `iam.tf`, `network.tf`, `variables.tf`, `outputs.tf`. There is no `encryption.tofu`: OpenTofu has no Azure Key Vault key provider, so state is encrypted at rest by Azure Storage only. One resource group per environment (AVM module), named `rg-<source>-<env>` unless `azure { resource-group … }` says otherwise. Globally unique names end in `substr(sha1("<subscription>/<resource group>"), 0, 6)`: the same on every run, no random resource, no guess. Every module gets `enable_telemetry = false`.

**Pins** (registry.opentofu.org and each module's `terraform.tf`, 2026-10-22; latest release of each). Providers: `azurerm` 4.81.0 (the newest 4.x; the key vault module needs `>= 4.81, < 5.0`, and azurerm 5.x is out but no module here allows it), `azapi` 2.13.0, `modtm` 0.4.0, `random` 3.9.1, `time` 0.14.2. `tofu init` resolves every module's ranges to exactly these.

| Neutral | Azure (AVM, `Azure/<name>/azurerm`) | As compiled |
|---|---|---|
| environment | `avm-res-resources-resourcegroup` 0.4.1 | one per environment; budget → `azurerm_consumption_budget_resource_group` (80 % forecast alert) |
| `infra.network` + `infra.subnet` | `avm-res-network-virtualnetwork` 0.22.3, `avm-res-network-natgateway` 0.3.2 | one subnet per tier (Azure subnets span every zone; the ranges are the first zone's of the AWS layout), no default outbound access; `nat` other than `none` → one zone-redundant StandardV2 NAT gateway on the private tier (`per-az` needs no more on Azure) |
| database tier | `avm-res-network-networksecuritygroup` 0.6.0, `avm-res-network-privatednszone` 0.5.0 | subnet delegated to PostgreSQL; NSG: 5432 from the subnets of functions with an edge to the database, anything within the subnet (HA), nothing else from the network; one `<env>-<net>.private.postgres.database.azure.com` zone linked to the VNet |
| `infra.bucket` | `avm-res-storage-storageaccount` 0.10.0 | StorageV2 ZRS, one container `data`; shared keys off, OAuth default, no public blobs, TLS 1.2, 7-day blob and container soft delete; `versioning` → blob versioning; `lifecycle` → a management policy rule on `data/<prefix>` (base blobs, and versions when versioned); `azure { access-tier }` |
| `infra.table` | `avm-res-documentdb-databaseaccount` 0.11.0 | one serverless NoSQL account per table (on-demand), a database and a container named after the node, partition key `/<partition-key>`; local auth off; `point-in-time-recovery` → continuous backup, 30 days; `deletion-protection` → a `CanNotDelete` lock; `ttl-attribute` → container TTL on |
| `infra.database` | `avm-res-dbforpostgresql-flexibleserver` 0.2.3 | major version only; `azure { sku-name }` (default `GP_Standard_D2ds_v5`); `storage-gb` rounded up to an offered size (commented); delegated subnet + private DNS zone, public access off, **`firewall_rules = {}`** (the module's default opens 0.0.0.0–255.255.255.255), Entra ID auth only (password auth off) with an Entra group as admin from variables; `high-availability` → zone-redundant HA; `deletion-protection` → lock |
| `infra.queue` / `infra.topic` | `avm-res-servicebus-namespace` 0.4.0 | one namespace per environment with queues or topics; Standard, Premium only when a key encrypts them; local auth off, TLS 1.2; `ordered` → sessions on queues, `support_ordering` on topics and sessions on their subscriptions; `retention-s` → TTL, `lock-timeout-s` → lock duration, `max-deliveries`, `dead-letter` → dead-lettering on expiry; topic → queue → a subscription with `forward_to`; topic → function → a subscription named after the function |
| `infra.key` | `avm-res-keyvault-vault` 0.12.0, `avm-res-managedidentity-userassignedidentity` 0.5.3 | premium vault per key, RBAC only, purge protection, `deletion-window-days` → soft-delete days; one RSA-HSM 3072 key (wrap/unwrap) rotated every 365 days, expiring after 730; the identity running tofu gets Key Vault Crypto Officer; a used key gets one identity that the stores encrypted with it share (Crypto Service Encryption User on the key) |
| `infra.function` | `avm-res-web-site` 0.24.1, `avm-res-web-serverfarm` 2.0.8, UAI module, storage module | per function: a user-assigned identity, a host storage account (identity-based `AzureWebJobsStorage`, deployment container), an FC1 plan (zone redundancy off: region-restricted), a Linux Function App on Flex Consumption: runtime from `runtime` (`nodejs22.x` → node 22, `python3.x`, `javaN`, `dotnetN` → dotnet-isolated), instance memory the smallest of 512/2048/4096 that holds `memory-mb` (or `azure { instance-memory-mb }`), HTTPS only, no basic-auth publishing; placement → VNet integration in the tier's subnet (delegated to `Microsoft.App/environments`) with all traffic routed through it; one Log Analytics workspace and Application Insights per environment with functions (`avm-res-operationalinsights-workspace` 0.5.1, `avm-res-insights-component` 0.5.0), local auth off, telemetry sent with the function's identity |
| `infra.api` | `avm-res-apimanagement-service` 0.9.0 | API Management Consumption; one API at path `/<api id>`, one operation per route with its path parameters, CORS as an API policy; each function a backend called with a function key: a `random_password` set as a host key through `azapi_resource_action` and kept in a secret named value |

**Edges.** Identity is one user-assigned identity per function; every grant is a role assignment for it, scoped to the one resource (`iam.tf`).

- `data` to a bucket: Storage Blob Data Reader for `read`; `write` and `read-write` get custom roles (`azurerm_role_definition`, scoped to the resource group) with exactly blobs write/add (and read), since the built-in Contributor also deletes. The scope is the container; `prefix` becomes an ABAC condition on the blob path.
- to a table: Cosmos DB Built-in Data Reader (`read`) or Contributor (`read-write`), a custom SQL role for `write` (create, replace, upsert, delete), scoped to the container.
- to a queue or topic: Azure Service Bus Data Sender and/or Receiver on the queue or topic.
- to a database: no role; the function signs in to Postgres as its identity, and the brief asks for a migration run by the Entra admin (`pgaadauth_create_principal`). The NSG rule above is the network half.
- queue → function: Receiver on the queue, plus the app settings the Service Bus trigger reads (`SERVICEBUS__fullyQualifiedNamespace`, `__credential`, `__clientId`, `<QUEUE>_QUEUE`). topic → function: Receiver on its subscription.
- store → key: customer-managed key through the key's identity (storage, Cosmos DB and Service Bus by key name, Postgres by the versionless key URI); the store waits for the role assignment.
- API → function: an operation and a backend (above). Function → network: VNet integration.
- Every function also gets Storage Blob Data Owner on its host storage and Monitoring Metrics Publisher on Application Insights.

**Gaps** (`rhizo_kit::infra::azure::gaps`, findings of `rhizo check`; warnings also become open questions in the brief):
- Errors: an `aws.*` type; a runtime Flex Consumption lacks (Ruby, custom); `memory-mb` over 4096; `lock-timeout-s` over 300 (Service Bus maximum); `backup-days` under 7; high availability on a burstable SKU; a database and a function in one subnet tier (one delegation per subnet); queues and topics encrypted with different keys (one namespace, one key); a greedy `{x+}` route.
- Warnings: `flow-logs` (on by default: VNet flow logs need the subscription's Network Watcher and a log storage account, so they are not compiled); `handler` (Azure Functions find entry points in code); `dead-letter=#false` (Service Bus still dead-letters after max deliveries); `ttl-attribute` (Cosmos DB expires by a `ttl` field in seconds, not an epoch attribute); `sort-key` (no sort key in Cosmos DB); a minor `engine-version`; `budget-monthly` (billing currency, not necessarily USD); a block for another cloud (`[infra.cloud.setting]`, also on aws).
- Not findings, but in the brief: stores accept Entra ID requests from any network (like S3 and DynamoDB endpoints); private endpoints are an open question. `timeout-s` becomes `functionTimeout` in `host.json`, a criterion of the implement task. `[infra.sqs.visibility]` does not apply on azure: the Functions host renews a message's lock while it runs.
- Function code is still the gap: an implement task per function, published after apply (`func azure functionapp publish`). The apply task lists the variables, `az login`/`az account set`, and the roles the deployer needs (Owner, or Contributor plus Role Based Access Control Administrator).

**L1 for azapi.** Most AVM modules are built on azapi, whose resources share one OpenTofu type, so the provider profile cannot type their IDs, and modules validate the IDs they are handed. `crates/iac/src/mocks.rs` now writes one `override_resource` per `azapi_resource` in the root module and every installed module: an ID of the resource's ARM type when its `type` is a literal, a variable's default or an `optional(string, "…")` default, and an `output` holding every attribute the module reads from it (null leaves; an empty object where a `for` iterates it). azurerm example IDs with `{placeholders}` are filled, and `versionless_id` is mocked as the ID without its version. Proof: the compiled example (86 resource instances) imports with `L1 equal`, and the existing corpus still passes.

**Not verified without a subscription.** `tofu validate` and the mocked plan check types, references and module validations, not Azure itself. Only an apply shows: region and SKU availability (StandardV2 NAT, Flex Consumption runtimes and VNet integration, PostgreSQL 17, serverless Cosmos DB with zone redundancy, continuous backup and CMK together); whether the host-key call succeeds before code is published; RBAC propagation delays (role assignments are created before the stores and apps that need them, but Entra can lag); the NSG on the delegated database subnet together with zone-redundant HA; the cost of Premium Service Bus (hundreds of USD a month) when a key encrypts queues.

## 2b. The Google Cloud compiler (built, step 4, 2026-10-22)

An environment with `cloud="gcp"` compiles to `infra/<env>/` from `crates/compile/src/infra/gcp.rs`; the aws and azure output is unchanged (byte-for-byte baseline of every example). As on azure, only neutral types compile: an `aws.*` node is an error naming it (`[infra.cloud.type]`), and `infra.resource`/`infra.module` compile verbatim.

**Project.** rhizo deploys into an existing project and creates none. The project is the `project_id` variable (validated, no default), not a diagram setting: like `aws_account_id` and `subscription_id`, it is never guessed, and the same environment can be applied to a sandbox project. The environment's `gcp` block holds only remote state (`state-bucket`, `state-prefix`, default `<source>/<env>`). The project's APIs are enabled by the `project_services` submodule with `disable_services_on_destroy = false`; every module and resource takes `project_id = local.project`, which is that module's output, so it waits for the APIs.

**Files.** `versions.tf` (OpenTofu `>= 1.11.1, < 2.0.0`, providers pinned exactly, a `gcs` backend when the `gcp` block names a state bucket), `encryption.tofu` with it (OpenTofu's `gcp_kms` key provider, key from the `state_kms_key` variable: unlike Azure, state and plans are encrypted client-side as on AWS), `providers.tf` (`google` and `google-beta` with `default_labels` `rhizo-source`, `rhizo-environment`, `managed-by`; `add_terraform_attribution_label = false`), `main.tf`, `iam.tf`, `variables.tf`, `outputs.tf`. There is no `network.tf`: firewall rules are inputs of the network module. Bucket names are global and end in `substr(sha1(var.project_id), 0, 6)`.

**Pins** (registry.opentofu.org and each module's `versions.tf`, 2026-10-22; latest release of each). Providers: `google` and `google-beta` 7.46.1 (the newest 7.x: google 8.x is out, but the pubsub module and others allow `< 8`, and the postgresql module needs `>= 7.22`), `null` 3.3.2 and `random` 3.9.1 (the sql modules, only with a database). `tofu init` resolves every module's ranges to exactly these. Telemetry: the modules declare `provider_meta "google" { module_name = "blueprints/terraform/…" }`, which the provider sends in its User-Agent; a caller cannot turn that off. The provider's own `goog-terraform-provisioned` label is off.

| Neutral | Google (`terraform-google-modules/<name>/google`) | As compiled |
|---|---|---|
| environment | `project-factory//modules/project_services` 18.4.0 | the APIs the environment uses; service agents (`activate_api_identities`) for services that use a customer-managed key; budget → `google_billing_budget` on the `billing_account_id` variable for the project (80 % forecast, email channels from `budget_alert_emails`, the billing account's currency) |
| `infra.network` + `infra.subnet` | `network` 18.3.1, `cloud-router` 9.1.0, `sql-db//modules/private_service_access` 28.3.0 | custom-mode VPC, regional routing; one regional subnet per tier (public and private, the first zone's ranges of the AWS layout), Private Google Access on each; `flow-logs` → subnet flow logs (5 s, 50 %, all metadata; native on GCP, unlike Azure); `nat` other than `none` → one Cloud Router with Cloud NAT for the private subnet (`per-az` needs no more: Cloud NAT is regional). The database tier is the private services access range (`<cidr>/len+4` at index 8 of the network, e.g. `10.30.8.0/24`: at least /24 for any allowed network size), peered for Cloud SQL, `deletion_policy = "ABANDON"` |
| database reach | network module `egress_rules` | allow tcp 5432 to the range from the network tag of each function with an edge to a database (priority 1000), deny everything else to it (65000) |
| `infra.bucket` | `cloud-storage//modules/simple_bucket` 12.4.1 | regional, uniform bucket-level access, public access prevention enforced, 7-day soft delete; `versioning`; `lifecycle` → Delete by age and prefix, noncurrent versions likewise, abort incomplete uploads after 7 days; `gcp { storage-class }` |
| `infra.table` | no module: raw `google_firestore_database`, `google_firestore_field` | one Firestore Native database per table, regional, billed per operation (on-demand); `point-in-time-recovery` → PITR (7 days); `deletion-protection` → delete protection; `ttl-attribute` → a TTL policy on the field in the collection named after the node. Firestore, not Bigtable: Bigtable is provisioned nodes (on-demand has no equivalent) and its row key is the only index |
| `infra.database` | `sql-db//modules/postgresql` 28.3.0 | `POSTGRES_<major>`, Enterprise edition (Enterprise Plus for `db-perf-optimized-*` tiers), `gcp { tier }` (default `db-custom-1-3840`, a dedicated core), `storage-gb` → disk; private IP only (`ipv4_enabled = false`) in the range above, `ENCRYPTED_ONLY`; no password users (`enable_default_user = false`), `cloudsql.iam_authentication` on, an IAM group from the `<db>_admin_group` variable as `cloudsqlsuperuser`, each function with an edge an IAM service-account user; `high-availability` → `REGIONAL`; backups with PITR, `backup-days` daily backups kept, transaction logs for min(backup-days, 7) days; `deletion-protection` → both Terraform and API deletion protection |
| `infra.queue` / `infra.topic` | `pubsub` 8.8.1 | a topic is a topic; a queue is a topic plus one subscription with its settings: `lock-timeout-s` → ack deadline, `retention-s` → message retention, `ordered` → message ordering, `dead-letter` → a dead-letter topic (a second module call, with a pull subscription keeping 14 days) after `max-deliveries`; subscriptions never expire; `grant_token_creator = false` (no project-level role) |
| `infra.key` | `kms` 4.2.1 | a key ring per key in the environment's region (CMEK needs the store's location), one HSM key rotated yearly, `deletion-window-days` → destroy-scheduled duration, `prevent_destroy` (key rings and keys cannot be deleted) |
| `infra.function` | no module: raw `google_cloudfunctions2_function`; `service-accounts` 5.1.0, `simple_bucket`, `google_artifact_registry_repository` | Cloud Run functions (2nd gen): `runtime` → `nodejs22`, `python312`, `java21`, `dotnet8`, `ruby33`; `handler` → entry point (its last part); `memory-mb` → the smallest size that holds it; `timeout-s`; `gcp { max-instance-count }` (default 100), min 0; code from `var.<fn>_package` uploaded into one sources bucket per environment, its generation in the build source so a new upload rebuilds; built by a dedicated builder service account into the environment's Artifact Registry repository; placement → Direct VPC egress in the tier's subnet with all traffic and a network tag; ingress internal only, all (authenticated) when an API routes to it. The GoogleCloudPlatform `cloud-functions` module was not used: it has VPC connectors only (a managed VM group) and defaults to one warm instance; the `cloud-run` v2 module needs a container image, so runtime and handler would mean nothing |
| `infra.api` | no module: raw `google_api_gateway_api`, `_api_config`, `_gateway` (google-beta) | an OpenAPI 2.0 document built with `yamlencode` from the routes: one operation per method and path with its path parameters, `x-google-backend` to the function URL with `APPEND_PATH_TO_ADDRESS` and a deadline of min(timeout-s, 600); the gateway's own service account calls the functions; the API's managed service is enabled; `gcp { region }` for the gateway (API Gateway runs in ten regions). API Gateway, not a global HTTPS load balancer with serverless NEGs: a load balancer routes by path only (not method), needs a certificate and domain, and leaves invocation to `allUsers` or IAP |

**Edges.** One service account per function (`module.service_accounts`, account id `<env>-<function>`); every grant is an IAM member binding for it on the one resource (`iam.tf`). Where a service has no resource-level binding, the binding is on the project with an IAM condition naming the resource.

- `data` to a bucket: `roles/storage.objectViewer` for read, `roles/storage.objectCreator` for write, both for read-write, on the bucket; never delete (so a write cannot overwrite an object: a criterion of the function's task). `prefix` → an IAM condition `resource.name.startsWith("projects/_/buckets/<bucket>/objects/<prefix>")`.
- to a table: `roles/datastore.viewer` (read), `roles/datastore.user` (read-write), a custom project role for write (create, update, delete, get metadata); on the project with a condition on `projects/<p>/databases/<db>`.
- to a queue or topic: `roles/pubsub.publisher` on the topic for write; `roles/pubsub.subscriber` on each of the queue's subscriptions for read.
- to a database: `roles/cloudsql.client` and `roles/cloudsql.instanceUser`, conditioned on the instance, plus the IAM database user; the SQL privileges are a migration by the admin group, a criterion of the function's task. The firewall rule above is the network half.
- queue → function: the queue's subscription is a push subscription to the function's URL with an OIDC token of one `pubsub-push` service account, which gets `roles/run.invoker` on that function's Cloud Run service. Push, not Eventarc: Eventarc creates its own subscription, so the queue's ack deadline, ordering, dead-letter topic and max deliveries would sit on a subscription nobody reads.
- topic → function: a push subscription on the topic, as above, ack deadline min(timeout-s, 600). topic → queue: Pub/Sub cannot forward between topics, so the queue gets a second subscription, on the topic, with the queue's settings and delivery (a warning).
- store → key: `roles/cloudkms.cryptoKeyEncrypterDecrypter` on the key for the store's service agent (Cloud Storage's from `google_storage_project_service_account`, Pub/Sub's, Cloud SQL's and Firestore's from `project_services`); the store waits for the binding.
- API → function: an operation per route, and `roles/run.invoker` for the gateway's service account. Function → network: Direct VPC egress.
- The builder gets `roles/storage.objectViewer` on the sources bucket, `roles/artifactregistry.writer` on the repository and `roles/logging.logWriter` on the project (logs have no resource-level binding).

**Gaps** (`rhizo_kit::infra::gcp::gaps`; warnings also become open questions in the brief):
- Errors: an `aws.*` type; a runtime Cloud Run functions lacks (custom `provided.*`); `lock-timeout-s` outside 10–600 (the ack deadline); `max-deliveries` outside 5–100 with a dead-letter topic; `retention-s` under 600; a function in the database tier (it is the private services access range); an API whose gateway region API Gateway does not serve; a `$default` route or a greedy `{x+}` parameter (API Gateway serves the paths in its document); a queue that triggers more than one function, or triggers one and is read by another (a push subscription has one endpoint and cannot be pulled).
- Warnings: a sort key (Firestore finds documents by id, so a partition key alone is the document id and needs no warning); `ttl-attribute` (a timestamp, not epoch seconds); a minor `engine-version`; `backup-days` over 7 (PITR reaches back 7 days on Enterprise); `cors-origins` (API Gateway has no CORS policy: `allowCors` passes preflights to the functions, which get `CORS_ALLOWED_ORIGINS` and a criterion); a routed or subscribed function with `timeout-s` over 600 (API Gateway and push wait at most 600 s); topic → queue (above); `budget-monthly` (billing currency); another cloud's block (`[infra.cloud.setting]`).
- Not findings: `[infra.lambda.egress]` does not apply on gcp, since every subnet has Private Google Access; `[infra.sqs.visibility]` does (the ack deadline must cover the function's timeout, as the visibility timeout must on AWS). Stores accept IAM-authorized requests from any network; a VPC Service Controls perimeter is an open question in the brief.
- Function code is still the gap: an implement task per function (a source zip whose path is `var.<fn>_package`, the entry point, the environment variables, push request handling, CORS, the database role). The apply task lists the variables, `gcloud auth application-default login`, the existing project, and the roles the deployer needs (Owner, or Editor plus Security Admin and Service Account User; Billing Account Costs Manager for the budget).

**L1.** The compiled example (91 resource instances, 11 outputs) imports with `L1 equal`. The google mock profile grew three generic rules in `crates/iac/src/mocks.rs`: a Compute Engine `self_link` of its type's shape (`google_compute_network` → `…/projects/mock/global/networks/mock`, regional types under `regions/us-central1`), since the sql module validates the network link; `google_compute_zones` curated with three zones, since the sql module indexes `names[0]`; and an `override_resource` per `google_api_gateway_api` that does not set `managed_service` (it is optional and computed, so the profile cannot mock it, and it is validated as a service name where it is enabled). The rules also apply to `google-beta`. The existing corpus still passes.

**Not verified without a project.** `tofu validate` (one deprecation warning, inside the network module) and the mocked plan check types, references and module validations, not Google Cloud. Only an apply shows: whether Pub/Sub push counts as internal ingress (it does in the documentation for the same project) and whether push needs `roles/iam.serviceAccountTokenCreator` for the Pub/Sub agent on projects created before April 2021; whether the three builder roles suffice for Cloud Build (Google documents `roles/cloudbuild.builds.builder` as the simple choice); Direct VPC egress with network tags and the egress firewall rules towards the peered range; `database_roles = ["cloudsqlsuperuser"]` for an IAM group; IAM conditions on Firestore database and Cloud SQL instance names; CMEK service agents created by `activate_api_identities` before their first use, and Firestore CMEK with an HSM key in the region; API Gateway with `allowCors` and the function URL as JWT audience; the budget API with user credentials (`user_project_override`); deletion: the peering is abandoned, keys are protected, and key ring, custom role and Firestore database names cannot be reused right after a destroy.

## 3. Import across clouds

Phase 1 already imports Azure and Google code as generic nodes; L1 with mock profiles (§1) makes the round trip verifiable for them. Folding into types (`iac-import.md` phase 2) then works per cloud: `azurerm_storage_account` plus `azurerm_storage_container` become an `infra.bucket` on an Azure environment. Built (step 5, 2026-10-22): see `iac-import.md`, "Phase 2 as built".

## Order

1. **L1 on mocks (done 2026-10-22).** Replace the fake-credential plan in `crates/iac` with `tofu test` and generated mock profiles for aws, azurerm and google. Add the Azure ID table generator. Corpus: rhizo's own `shop` output, plus Azure and Google entries. Exit: the corpus passes with network denied after init.
2. **Neutral types, AWS first (done 2026-10-22).** `cloud` on the environment, neutral types and props, the AWS compiler reusing today's emitters. Exit: the baseline is unchanged; neutral `shop` compiles to the same resources as the `aws.*` `shop`.
3. **Azure compiler (done 2026-10-22, §2a).** Azure Verified Modules pinned in the catalog; edges as role assignments and managed identities (private endpoints left as an open question). Exit: `tofu validate` and an L1 mock plan pass on `examples/playground/12-azure-infra.rhizo`.
4. **Google compiler (done 2026-10-22, §2b).** terraform-google-modules pinned in the catalog; edges as IAM bindings on each resource and one service account per function (VPC Service Controls left as an open question). Exit: `tofu validate` and an L1 mock plan pass on `examples/playground/13-gcp-infra.rhizo`.
5. **Folding imports (done 2026-10-22)** into neutral types per cloud (`iac-import.md` phase 2). Exit: every corpus entry still L1 equal and idempotent; rhizo's own output for each cloud folds back to the neutral types it was compiled from, gaps named.
6. **Adopt (done 2026-10-22)**: folded nodes switch to rhizo's code, each cloud's emitter writing one node into a preserved root module (`rhizo.tf`, with `moved` blocks), proven offline by L1-adopt and live by a person's `tofu plan` (`iac-import.md`, "Adopt (switching shapes) as built"). Exit: rhizo's own output for each cloud, imported and adopted with `--all`, plans with no attribute difference and passes `tofu validate`; a raw S3 bucket with satellites is accounted for through `moved`. Not yet: raw Azure and Google resources.

## Open questions

- Should a project be able to target several clouds at once (multi-cloud), or is one cloud per environment enough? This design allows the second, which covers both.
- Which neutral props deserve a place? Start with the ones every cloud has, and let corpus imports show what is missing.
- Q9 (catalog admission policy) now covers three module families. Azure Verified Modules has a formal support statement; terraform-google-modules is maintained by Google; both still need exact pins.
