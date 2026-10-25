# Requires a provider outside rhizo's allow-list: the import refuses it
# before OpenTofu downloads it, unless --allow-provider integrations/github.
terraform {
  required_providers {
    github = {
      source  = "integrations/github"
      version = "~> 6.0"
    }
  }
}

resource "github_repository" "docs" {
  name       = "docs"
  visibility = "private"
}
