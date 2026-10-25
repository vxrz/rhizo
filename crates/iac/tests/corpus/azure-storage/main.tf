data "azurerm_client_config" "current" {}

resource "azurerm_resource_group" "shop" {
  name     = "rg-shop"
  location = var.location
}

resource "azurerm_virtual_network" "shop" {
  name                = "vnet-shop"
  location            = azurerm_resource_group.shop.location
  resource_group_name = azurerm_resource_group.shop.name
  address_space       = ["10.20.0.0/16"]
}

resource "azurerm_subnet" "private" {
  name                 = "snet-private"
  resource_group_name  = azurerm_resource_group.shop.name
  virtual_network_name = azurerm_virtual_network.shop.name
  address_prefixes     = ["10.20.1.0/24"]
}

resource "azurerm_storage_account" "shop" {
  name                          = "stshopreceipts"
  resource_group_name           = azurerm_resource_group.shop.name
  location                      = azurerm_resource_group.shop.location
  account_tier                  = "Standard"
  account_replication_type      = "ZRS"
  min_tls_version               = "TLS1_2"
  public_network_access_enabled = false

  blob_properties {
    versioning_enabled = true
  }
}

resource "azurerm_storage_container" "this" {
  for_each           = var.containers
  name               = each.key
  storage_account_id = azurerm_storage_account.shop.id
}

resource "azurerm_private_endpoint" "blob" {
  name                = "pe-shop-blob"
  location            = azurerm_resource_group.shop.location
  resource_group_name = azurerm_resource_group.shop.name
  subnet_id           = azurerm_subnet.private.id

  private_service_connection {
    name                           = "blob"
    private_connection_resource_id = azurerm_storage_account.shop.id
    subresource_names              = ["blob"]
    is_manual_connection           = false
  }
}

resource "azurerm_user_assigned_identity" "orders" {
  name                = "id-orders"
  location            = azurerm_resource_group.shop.location
  resource_group_name = azurerm_resource_group.shop.name
}

resource "azurerm_role_assignment" "orders_read" {
  scope                = azurerm_storage_container.this["receipts"].id
  role_definition_name = "Storage Blob Data Reader"
  principal_id         = azurerm_user_assigned_identity.orders.principal_id
}

resource "azurerm_key_vault" "shop" {
  name                      = "kv-shop"
  location                  = azurerm_resource_group.shop.location
  resource_group_name       = azurerm_resource_group.shop.name
  tenant_id                 = data.azurerm_client_config.current.tenant_id
  sku_name                  = "standard"
  purge_protection_enabled  = true
  enable_rbac_authorization = true
}

resource "azurerm_key_vault_key" "data" {
  name         = "data"
  key_vault_id = azurerm_key_vault.shop.id
  key_type     = "RSA"
  key_size     = 3072
  key_opts     = ["wrapKey", "unwrapKey"]
}

output "storage_account_id" {
  value = azurerm_storage_account.shop.id
}
