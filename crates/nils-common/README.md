# nils-common

## Overview
`nils-common` is a small shared library crate for cross-CLI helpers within this workspace.

It is intentionally minimal and can grow as shared needs emerge.

## Example
```rust
let greeting = nils_common::greeting("Nils");
assert_eq!(greeting, "Hello, Nils!");
```

