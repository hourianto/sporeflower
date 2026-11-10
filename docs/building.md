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

Native builds and releases on Linux, macOS, and Windows use mimalloc by default.
Building them requires a C/C++ compiler. To use the system allocator instead:

```sh
cargo build --release --locked --no-default-features
```

Mimalloc defaults to a 64 MiB arena reserve, a 512 KiB arena object limit,
a 10 ms purge delay, and disabled transparent huge pages. Explicit
`MIMALLOC_ARENA_RESERVE`, `MIMALLOC_ARENA_MAX_OBJECT_SIZE`,
`MIMALLOC_PURGE_DELAY`, and `MIMALLOC_ALLOW_THP` environment variables override
these settings. Use size suffixes for sizes, for example `64MiB` or `512KiB`.
The WebAssembly build uses its existing allocator.

[Running the tests](testing.md) requires Java tooling. Browser integrations use
the separate [WebAssembly build](../wasm/README.md).
