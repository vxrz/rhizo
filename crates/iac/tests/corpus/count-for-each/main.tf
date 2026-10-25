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

provider "aws" {
  alias  = "us"
  region = "us-east-1"
}

variable "queues" {
  type = map(object({
    visibility = number
    fifo       = optional(bool, false)
  }))
  default = {
    orders   = { visibility = 60, fifo = true }
    emails   = { visibility = 30 }
    invoices = { visibility = 120 }
  }
}

variable "alarm_emails" {
  type    = list(string)
  default = ["ops@example.com", "oncall@example.com"]
}

variable "with_dlq" {
  type    = bool
  default = true
}

resource "aws_sqs_queue" "dlq" {
  count = var.with_dlq ? 1 : 0
  name  = "dead-letters"
}

resource "aws_sqs_queue" "queue" {
  for_each = var.queues

  name                       = each.value.fifo ? "${each.key}.fifo" : each.key
  fifo_queue                 = each.value.fifo
  visibility_timeout_seconds = each.value.visibility
  redrive_policy = var.with_dlq ? jsonencode({
    deadLetterTargetArn = aws_sqs_queue.dlq[0].arn
    maxReceiveCount     = 5
  }) : null
}

resource "aws_sns_topic" "alarms" {
  provider = aws.us
  name     = "alarms"
}

resource "aws_sns_topic_subscription" "email" {
  provider  = aws.us
  count     = length(var.alarm_emails)
  topic_arn = aws_sns_topic.alarms.arn
  protocol  = "email"
  endpoint  = var.alarm_emails[count.index]
}

output "queue_urls" {
  value = { for name, q in aws_sqs_queue.queue : name => q.url }
}
