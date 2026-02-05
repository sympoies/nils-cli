# nils-common

## Overview
`nils-common` is a small shared library crate for cross-CLI helpers within this workspace.

It is intentionally minimal and can grow as shared needs emerge.

## Example
```rust
let greeting = nils_common::greeting("Nils");
assert_eq!(greeting, "Hello, Nils!");
```

## Process helpers
```rust
let git = nils_common::process::find_in_path("git");
assert!(git.is_some());
```
