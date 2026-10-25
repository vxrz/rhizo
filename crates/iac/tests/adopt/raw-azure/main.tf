# Raw Azure resources: a Key Vault key rhizo can adopt into the key vault
# module (azurerm inside: a move within one type), and a storage account it
# refuses (its module is built on azapi: a move across types).
terraform {
  required_providers {
    azurerm = {
      source  = "hashicorp/azurerm"
      version = "~> 4.81"
    }
  }
}

provider "azurerm" {
  features {}
}

variable "tenant_id" {
  type    = string
  default = "00000000-0000-0000-0000-000000000000"
}

resource "azurerm_resource_group" "shop" {
  name     = "rg-shop"
  location = "westeurope"
}

resource "azurerm_key_vault" "shop" {
  name                       = "kv-acme-shop"
  location                   = azurerm_resource_group.shop.location
  resource_group_name        = azurerm_resource_group.shop.name
  tenant_id                  = var.tenant_id
  sku_name                   = "premium"
  rbac_authorization_enabled = true
  purge_protection_enabled   = true
  soft_delete_retention_days = 30

  tags = {
    team = "shop"
  }
}

resource "azurerm_key_vault_key" "data" {
  name         = "data"
  key_vault_id = azurerm_key_vault.shop.id
  key_type     = "RSA-HSM"
  key_size     = 3072
  key_opts     = ["unwrapKey", "wrapKey"]
}

resource "azurerm_storage_account" "assets" {
  name                     = "stacmeshopassets"
  resource_group_name      = azurerm_resource_group.shop.name
  location                 = azurerm_resource_group.shop.location
  account_tier             = "Standard"
  account_replication_type = "ZRS"
}

output "data_key" {
  value = azurerm_key_vault_key.data.versionless_id
}

output "vault_id" {
  value = azurerm_key_vault.shop.id
}

output "vault_uri" {
  value = azurerm_key_vault.shop.vault_uri
}
