---
title: Checkout
---

- Customer
- Web app
- Orders API
- Payments

## Connections

- Customer → Web app (Place order)
- Web app → Orders API (POST /orders)
- Orders API → Payments (Charge card)
- Payments → Orders API (charged)
- Orders API → Orders API (Save order)
- Orders API → Web app (201 Created)
- Web app → Customer (Confirmation)
