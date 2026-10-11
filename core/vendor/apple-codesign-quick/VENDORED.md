# apple-codesign-quick 0.1.0, vendored

From crates.io (https://github.com/Dadoum/apple-crates), LGPL-2.1-or-later, used through
`[patch.crates-io]` in core/Cargo.toml so isideload builds against the same copy.

One change: `Extensions/` (ExtensionKit extensions, e.g. UTM's iOSHelper.appex) is treated as a
nested-bundle folder like `PlugIns/`, so those extensions are signed and recorded in the parent's
CodeResources instead of being hashed as plain resources with their old signature
(src/bundle.rs: `sub_bundles` and `should_hash_resource`).
