# Import corpus

Root modules that `rhizo import tofu` must round-trip: import, compile, and
plan to the same result (L1). Written for rhizo and licensed like it; each
folder shows a construct real projects use:

| Folder | What it covers |
|---|---|
| `s3-bucket` | A raw S3 bucket with its satellite resources, a policy from a data source, an S3 backend, a lock file |
| `vpc-module` | A registry module call (terraform-aws-modules/vpc) and resources that use its outputs |
| `lambda-iam` | A Lambda function, its IAM role and policies referring to a bucket, `jsonencode`, `depends_on` |
| `count-for-each` | `count`, `for_each` over a map of objects, a conditional resource, an aliased provider |
| `dynamic-locals` | `dynamic` blocks, complex `locals` (for expressions, heredoc, `merge`), `lifecycle` |
| `moved-import` | `moved`, `import`, `removed` and `check` blocks, a `.tofu` file that replaces a `.tf` file, `terraform.tfvars` |
| `azure-storage` | azurerm: a storage account with containers (`for_each`), a VNet, subnet and private endpoint, a managed identity with a role assignment on a container, a key vault and key, `azurerm_client_config` |
| `gcp-storage` | google: a bucket encrypted with a Cloud KMS key, a service account with IAM members on the bucket and a Pub/Sub topic, a subscription, `google_project` |

**Semantic coverage** (`iac-import.md`, phase 2): the share of resources
and module calls (data sources do not count) that fold into a neutral type,
or whose meaning folds into one as a satellite. `corpus_round_trips` checks
each entry's report against this table, so a change in folding shows here.

| Folder | Folded | Satellites | Resources and module calls | Coverage |
|---|---|---|---|---|
| `azure-storage` | 4 | 1 | 10 | 50% |
| `count-for-each` | 3 | 0 | 4 | 75% |
| `dynamic-locals` | 0 | 0 | 2 | 0% |
| `gcp-storage` | 4 | 0 | 8 | 50% |
| `lambda-iam` | 2 | 0 | 6 | 33% |
| `moved-import` | 2 | 0 | 3 | 67% |
| `s3-bucket` | 1 | 6 | 7 | 100% |
| `vpc-module` | 1 | 0 | 3 | 33% |

What stays generic, by design: IAM roles, policies and bindings, security
groups, launch templates, resource groups, identities, private endpoints,
log groups and subscriptions are no neutral type; they stay verbatim nodes
and show up as edges (`access` on grants) where they join folded nodes.

L1 plans with every provider mocked (`src/mocks.rs`), so data sources that would call a cloud plan offline too. `rhizo_output_round_trips` in `../corpus.rs` also imports what rhizo compiles from `examples/shop`.
