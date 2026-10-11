# islas

Small backends compiled to WebAssembly, on one server of your own. Push a package, and the app answers on its own subdomain with its own SQLite database, key-value store, secrets, logs and limits.

This repository holds islas's releases and the source of the crates apps are built with. The platform's own source isn't public yet.

## Install

```sh
curl -fsSL https://github.com/isla-rs/isla-server/releases/latest/download/install.sh | sh
```

A given version: `curl -fsSL … | ISLAS_VERSION=v0.1.0 sh`. There are builds for Linux (x86_64 and aarch64, glibc 2.35 or later: Ubuntu 22.04, Debian 12 and later) and macOS (Intel and Apple Silicon). The script checks the download against the release's `SHA256SUMS` and installs `islas` into `/usr/local/bin` as root, else `~/.local/bin`; `ISLAS_BIN_DIR` picks another folder. Running it again updates `islas`.

## Start an app

```sh
islas new notes      # a project in ./notes; needs Rust from rustup, and Node for the page
cd notes
islas dev            # http://localhost:3000, rebuilt and reloaded as you save
```

The project's `README.md` and `CLAUDE.md` say the rest.

## What's in this repository

- `framework/`: `islas`, the crate apps depend on: routes and controllers, requests and responses, typed rows, cookies and sessions, native tests.
- `sdk/`: `islas-sdk`, the platform's interface as plain Rust functions.
- `wit/`: the interface itself, in WIT, on WASI 0.2.
- `crates/crypto/`: `islas-crypto`, the sealing and signing that the host and the SDK's native tests share.

Each release replaces these with the version it was built with and tags them, so an app made by `islas new` depends on its own release:

```toml
islas = { git = "https://github.com/isla-rs/isla-server", tag = "v0.1.0" }
```

They are copied here from where islas is developed; changes made in this repository would be overwritten by the next release.

## License

The crates in this repository are under the MIT license (`LICENSE`).
