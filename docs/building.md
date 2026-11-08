# Building

Building requires **Rust 1.88 or newer**. The native decompiler and mapping tools
do not need Java or a J2ME SDK.

From a checkout, install the command with Cargo:

```sh
cargo install --path . --locked
```

This installs `sporeflower` in Cargo's binary directory, normally `~/.cargo/bin`.
To build without installing:

```sh
cargo build --release --locked
```

The executable is `target/release/sporeflower` (`sporeflower.exe` on Windows).

The native CLI can optionally use mimalloc instead of the system allocator:

```sh
cargo build --release --locked --features mimalloc
```

This also requires a C/C++ compiler.

[Running the tests](testing.md) requires Java tooling. Browser integrations use
the separate [WebAssembly build](../wasm/README.md).
