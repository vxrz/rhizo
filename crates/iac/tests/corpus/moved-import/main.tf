provider "aws" {
  region = var.region
}

variable "region" {
  type = string
}

variable "queue_name" {
  type = string
}

# The topic was called old_events before; state moves with it.
moved {
  from = aws_sns_topic.old_events
  to   = aws_sns_topic.events
}

# An existing queue, adopted into state on the next apply.
import {
  to = aws_sqs_queue.legacy
  id = "https://sqs.eu-central-1.amazonaws.com/123456789012/legacy"
}

# No longer managed here, but left in place.
removed {
  from = aws_s3_bucket.reports

  lifecycle {
    destroy = false
  }
}

resource "aws_sqs_queue" "legacy" {
  name = var.queue_name
}

resource "aws_sns_topic" "events" {
  name = "events"
}

resource "aws_sns_topic_subscription" "legacy" {
  topic_arn = aws_sns_topic.events.arn
  protocol  = "sqs"
  endpoint  = aws_sqs_queue.legacy.arn
}

check "topic_name" {
  assert {
    condition     = aws_sns_topic.events.name == "events"
    error_message = "The events topic was renamed."
  }
}
