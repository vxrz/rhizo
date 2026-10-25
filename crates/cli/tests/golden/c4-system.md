---
title: Online shop: context, containers, components
---

## System context

- Customer
  > Browses the catalog, orders and pays
- Payment provider
- Email service

## Online shop

- Web shop
  > Single-page app in the browser
- Orders DB

## API

- Orders controller
- Checkout service
- Order repository

## Connections

- Orders controller → Checkout service
- Checkout service → Order repository
- Web shop → Orders controller (JSON over HTTPS)
- Order repository → Orders DB (SQL)
- Customer → Web shop (shops with)
- Checkout service → Payment provider (charges)
- Checkout service → Email service (sends receipts)
