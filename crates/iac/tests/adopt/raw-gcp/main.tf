# Raw Google Cloud resources: four kinds rhizo can adopt into its own code,
# and a topic with a subscription it leaves (rhizo's queue is another shape).
terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 7.0"
    }
  }
}

provider "google" {
  project = var.project
  region  = "europe-west3"
}

variable "project" {
  type    = string
  default = "acme-shop"
}

resource "google_storage_bucket" "assets" {
  name                        = "acme-shop-assets"
  location                    = "EU"
  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  public_access_prevention    = "enforced"

  versioning {
    enabled = true
  }

  labels = {
    team = "shop"
  }
}

resource "google_storage_bucket_iam_member" "assets_read" {
  bucket = google_storage_bucket.assets.name
  role   = "roles/storage.objectViewer"
  member = "group:shop@example.com"
}

resource "google_pubsub_topic" "events" {
  name = "shop-events"

  labels = {
    team = "shop"
  }
}

resource "google_pubsub_topic_iam_member" "events_publish" {
  topic  = google_pubsub_topic.events.id
  role   = "roles/pubsub.publisher"
  member = "group:shop@example.com"
}

resource "google_kms_key_ring" "shop" {
  name     = "shop"
  location = "europe-west3"
}

resource "google_kms_crypto_key" "data" {
  name            = "data"
  key_ring        = google_kms_key_ring.shop.id
  rotation_period = "31536000s"

  version_template {
    algorithm        = "GOOGLE_SYMMETRIC_ENCRYPTION"
    protection_level = "HSM"
  }
}

resource "google_kms_crypto_key_iam_member" "data_use" {
  crypto_key_id = google_kms_crypto_key.data.id
  role          = "roles/cloudkms.cryptoKeyEncrypterDecrypter"
  member        = "group:shop@example.com"
}

resource "google_firestore_database" "app" {
  name                              = "app"
  location_id                       = "eur3"
  type                              = "FIRESTORE_NATIVE"
  point_in_time_recovery_enablement = "POINT_IN_TIME_RECOVERY_ENABLED"
  delete_protection_state           = "DELETE_PROTECTION_ENABLED"
  deletion_policy                   = "DELETE"
}

resource "google_pubsub_topic" "jobs" {
  name = "shop-jobs"
}

resource "google_pubsub_subscription" "jobs_worker" {
  name                 = "shop-jobs-worker"
  topic                = google_pubsub_topic.jobs.id
  ack_deadline_seconds = 30
}

output "assets_url" {
  value = google_storage_bucket.assets.url
}

output "events_topic" {
  value = google_pubsub_topic.events.id
}

output "data_key" {
  value = google_kms_crypto_key.data.id
}

output "key_ring" {
  value = google_kms_key_ring.shop.id
}
