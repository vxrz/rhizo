data "google_project" "this" {}

resource "google_service_account" "orders" {
  account_id   = "orders"
  display_name = "Orders service"
}

resource "google_kms_key_ring" "shop" {
  name     = "shop"
  location = var.region
}

resource "google_kms_crypto_key" "data" {
  name            = "data"
  key_ring        = google_kms_key_ring.shop.id
  rotation_period = "7776000s"
}

resource "google_storage_bucket" "receipts" {
  name                        = "${var.project}-receipts-${data.google_project.this.number}"
  location                    = "EU"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  versioning {
    enabled = true
  }

  encryption {
    default_kms_key_name = google_kms_crypto_key.data.id
  }
}

resource "google_storage_bucket_iam_member" "orders_read" {
  bucket = google_storage_bucket.receipts.name
  role   = "roles/storage.objectViewer"
  member = "serviceAccount:${google_service_account.orders.email}"
}

resource "google_pubsub_topic" "orders" {
  name = "orders"
}

resource "google_pubsub_subscription" "email" {
  name                       = "email"
  topic                      = google_pubsub_topic.orders.id
  ack_deadline_seconds       = 30
  message_retention_duration = "604800s"
}

resource "google_pubsub_topic_iam_member" "orders_publish" {
  topic  = google_pubsub_topic.orders.id
  role   = "roles/pubsub.publisher"
  member = google_service_account.orders.member
}

output "bucket" {
  value = google_storage_bucket.receipts.url
}
