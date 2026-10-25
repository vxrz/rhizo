output "bucket" {
  description = "Bucket name"
  value       = aws_s3_bucket.receipts.id
}

output "bucket_arn" {
  value = aws_s3_bucket.receipts.arn
}
