## 2026-11-06 - Retain shared GnuPG daemons in marker inventories
<!-- devlog-id: 2026-11-06-shared-gnupg-daemon-close -->

### Result

- Marker-only close inventories retain reparented GnuPG daemons without
  treating them as session survivors.

### Why / context

- Signing can start shared per-user daemons that outlive the initiating
  session.

### Evidence

- TODO

### Links

- <https://github.com/sympoies/nils-cli/issues/3408>
