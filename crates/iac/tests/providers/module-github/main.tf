# Only the registry module requires integrations/github: the import refuses
# it after `tofu get` fetched the module's code, before any provider is
# downloaded, unless --allow-provider integrations/github.
module "team" {
  source  = "mineiros-io/team/github"
  version = "0.9.0"

  name = "platform"
}
