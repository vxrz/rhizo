variable "subscription_id" {
  type = string
}

variable "location" {
  type    = string
  default = "westeurope"
}

variable "containers" {
  type    = set(string)
  default = ["receipts", "exports"]
}
