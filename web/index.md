---
layout: home

hero:
  name: XavierDB - Just less than MongoDB
  image:
    src: /logo.png
    alt: XavierDB
  actions:
    - theme: brand
      text: Open GitHub
      link: https://github.com/ThomasByr/XavierDB
    - theme: alt
      text: Open admin dashboard
      link: https://api.xavierdb.fr/dashboard

features:
  - icon: ⚡
    title: REST over MongoDB
    details: Full CRUD on /q/&lt;db&gt;/&lt;coll&gt; — GET, POST, PUT, PATCH, DELETE.
  - icon: 🔑
    title: JWT auth
    details: Per-client tokens via /auth, Argon2id-hashed, throttled.
  - icon: 🛡️
    title: Granular permissions
    details: Per-app, per-collection rights in authorized_keys.yml.
  - icon: 🧭
    title: Keyset pagination
    details: Stable cursors, no offset drift.
  - icon: ⚖️
    title: Adaptive limits
    details: Per-app document caps that react to server pressure.
  - icon: 🖥️
    title: Admin dashboard
    details: Embedded SPA — metrics, perms editor, config undo/redo.
---
