terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "= 6.69.1"
    }
  }
}

provider "aws" {
  region = var.region
}

variable "region" {
  type    = string
  default = "eu-central-1"
}

variable "environment" {
  type    = string
  default = "prod"
}

data "aws_partition" "current" {}

resource "aws_s3_bucket" "artifacts" {
  bucket = "example-artifacts-${var.environment}"
}

data "aws_iam_policy_document" "assume_lambda" {
  statement {
    actions = ["sts:AssumeRole"]

    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "thumbnailer" {
  name               = "${var.environment}-thumbnailer"
  assume_role_policy = data.aws_iam_policy_document.assume_lambda.json
}

resource "aws_iam_role_policy" "thumbnailer_s3" {
  name = "artifacts"
  role = aws_iam_role.thumbnailer.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = ["s3:GetObject", "s3:PutObject"]
        Resource = "${aws_s3_bucket.artifacts.arn}/thumbnails/*"
      },
    ]
  })
}

resource "aws_iam_role_policy_attachment" "thumbnailer_logs" {
  role       = aws_iam_role.thumbnailer.name
  policy_arn = "arn:${data.aws_partition.current.partition}:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

resource "aws_cloudwatch_log_group" "thumbnailer" {
  name              = "/aws/lambda/${var.environment}-thumbnailer"
  retention_in_days = 30
}

resource "aws_lambda_function" "thumbnailer" {
  function_name = "${var.environment}-thumbnailer"
  role          = aws_iam_role.thumbnailer.arn
  s3_bucket     = aws_s3_bucket.artifacts.id
  s3_key        = "releases/thumbnailer.zip"
  handler       = "index.handler"
  runtime       = "nodejs22.x"
  architectures = ["arm64"]
  memory_size   = 512
  timeout       = 15

  environment {
    variables = {
      BUCKET = aws_s3_bucket.artifacts.id
      PREFIX = "thumbnails/"
    }
  }

  depends_on = [
    aws_iam_role_policy_attachment.thumbnailer_logs,
    aws_cloudwatch_log_group.thumbnailer,
  ]
}

output "function_arn" {
  value = aws_lambda_function.thumbnailer.arn
}
