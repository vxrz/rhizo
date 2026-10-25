# Raw resources of four kinds rhizo can adopt into its modules.
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

resource "aws_kms_key" "events" {
  description             = "Order events"
  deletion_window_in_days = 30
  enable_key_rotation     = true
}

resource "aws_sns_topic" "events" {
  name              = "orders-events"
  kms_master_key_id = aws_kms_key.events.arn
}

resource "aws_sqs_queue" "jobs" {
  name                       = "orders-jobs"
  message_retention_seconds  = 345600
  visibility_timeout_seconds = 60
  sqs_managed_sse_enabled    = true
}

resource "aws_dynamodb_table" "carts" {
  name         = "orders-carts"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "cart_id"

  attribute {
    name = "cart_id"
    type = "S"
  }

  point_in_time_recovery {
    enabled = true
  }

  deletion_protection_enabled = true
}

output "topic_arn" {
  value = aws_sns_topic.events.arn
}

output "queue_url" {
  value = aws_sqs_queue.jobs.url
}

output "table" {
  value = aws_dynamodb_table.carts.name
}
