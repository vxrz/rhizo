terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "= 6.69.1"
    }
  }
}

provider "aws" {
  region = "eu-central-1"
}

locals {
  name = "shop"
  cidr = "10.20.0.0/16"
  azs  = ["eu-central-1a", "eu-central-1b", "eu-central-1c"]
  tags = {
    Project = local.name
  }
}

module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "6.7.3"

  name = "${local.name}-vpc"
  cidr = local.cidr
  azs  = local.azs

  private_subnets  = [for i, _ in local.azs : cidrsubnet(local.cidr, 4, i)]
  public_subnets   = [for i, _ in local.azs : cidrsubnet(local.cidr, 8, i + 48)]
  database_subnets = [for i, _ in local.azs : cidrsubnet(local.cidr, 8, i + 64)]

  create_database_subnet_group = true
  enable_nat_gateway           = true
  single_nat_gateway           = true
  enable_dns_hostnames         = true

  tags = local.tags
}

resource "aws_security_group" "app" {
  name_prefix = "${local.name}-app-"
  description = "Application servers"
  vpc_id      = module.vpc.vpc_id

  tags = local.tags
}

resource "aws_vpc_security_group_egress_rule" "app_https" {
  security_group_id = aws_security_group.app.id
  description       = "HTTPS out"
  cidr_ipv4         = "0.0.0.0/0"
  ip_protocol       = "tcp"
  from_port         = 443
  to_port           = 443
}

output "vpc_id" {
  value = module.vpc.vpc_id
}

output "private_subnets" {
  value = module.vpc.private_subnets
}
