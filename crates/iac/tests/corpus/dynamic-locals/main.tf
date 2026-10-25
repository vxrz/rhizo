terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "= 6.69.1"
    }
  }
}

provider "aws" {
  region = "eu-west-1"
}

variable "environment" {
  type    = string
  default = "staging"
}

variable "extra_tags" {
  type    = map(string)
  default = { CostCenter = "web" }
}

locals {
  name = "web-${var.environment}"
  ingress = [
    { port = 443, cidrs = ["0.0.0.0/0"], description = "HTTPS" },
    { port = 80, cidrs = ["0.0.0.0/0"], description = "HTTP redirect" },
    { port = 22, cidrs = ["10.0.0.0/8", "192.168.0.0/16"], description = "SSH from office" },
  ]
  ports_by_name = { for r in local.ingress : lower(replace(r.description, " ", "-")) => r.port }
  tags = merge(var.extra_tags, {
    Environment = var.environment
    Name        = local.name
  })
  user_data = <<-EOT
    #!/bin/sh
    echo "starting ${local.name}" > /var/log/boot.log
    %{for name, port in local.ports_by_name~}
    echo "${name}=${port}" >> /etc/ports
    %{endfor~}
  EOT
}

resource "aws_security_group" "web" {
  name        = local.name
  description = "Web tier"

  dynamic "ingress" {
    for_each = local.ingress
    iterator = rule
    content {
      description = rule.value.description
      from_port   = rule.value.port
      to_port     = rule.value.port
      protocol    = "tcp"
      cidr_blocks = rule.value.cidrs
    }
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = local.tags

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_launch_template" "web" {
  name_prefix            = "${local.name}-"
  image_id               = "ami-0123456789abcdef0"
  instance_type          = var.environment == "prod" ? "m7g.large" : "t4g.small"
  user_data              = base64encode(local.user_data)
  vpc_security_group_ids = [aws_security_group.web.id]

  dynamic "tag_specifications" {
    for_each = toset(["instance", "volume"])
    content {
      resource_type = tag_specifications.value
      tags          = local.tags
    }
  }

  lifecycle {
    ignore_changes = [image_id]
  }
}

output "ports" {
  value = local.ports_by_name
}
