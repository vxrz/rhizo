# Terraform reads this file; OpenTofu reads versions.tofu instead.
terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 6.0"
    }
  }
}
