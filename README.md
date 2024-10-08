<div align="center">
  <h1><code>tritongue</code></h1>

  <p>
    <strong>Matrix bots in Rust and WebAssembly</strong>
  </p>

  <p>
    <a href="https://github.com/hotsphink/tritongue/actions?query=workflow%3ARust"><img src="https://github.com/hotsphink/tritongue/workflows/Rust/badge.svg" alt="build status" /></a>
    <img src="https://img.shields.io/badge/rustc-stable+-green.svg" alt="supported rustc stable" />
  </p>
</div>

## TL;DR

Tritongue is a fork of bnjbvr's excellent <a
href="https://github.com/bnjbvr/trinity/">Trinity</a> Matrix bot framework.
Trinity is an experimental bot framework written in Rust and using
matrix-rust-sdk, as well as commands / modules compiled to WebAssembly, with
convenient developer features like modules hot-reload. Tritongue goes in a
somewhat different direction, adding in the ability to write modules in Python
while hopefully not breaking the WebAssembly stuff in the process.

## What is this?

See the [Trinity README](https://github.com/bnjbvr/trinity/blob/main/README.md)
for a proper description of the main bot framework, which began as bnjbvr's
weekend project.

<em>My</em> weekend project was to write my own bot from scratch in Rust based
on the examples in
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk). It worked, it
was gratifying to setup, but I realized there was a <em>lot</em> of polish
required to make it real, so I checked out Trinity. (Which was where I had
heard about matrix-rust-sdk in the first place!) It was clearly far ahead of
mine, and written better besides (by someone who understands both Matrix and
Rust far better than I do), so it seemed a better starting point.

But I also knew that I wanted to do a lot of messing around and trying out some
very opinionated ways of doing things, which at the moment is feeling like a
good enough reason to fork rather than contribute back. Also, Trinity is not
being actively developed right now, and I didn't want to trick bnjbvr into
spending time maintaining it in the face of my changes.

If it works out, I will also be adding back in various Mozilla-specific
functionality from my past bot frameworks, in particular mrgiggles.

I will try to maintain Trinity support for as long as I can, especially in
anything related to WebAssembly. That means the code will more often say
"trinity" than "tritongue".

From the Trinity README:

Bot commands can be implemented as WebAssembly components, using
[Wasmtime](https://github.com/bytecodealliance/wasmtime) as the WebAssembly virtual machine, and
[wit-bindgen](https://github.com/bytecodealliance/wit-bindgen) for conveniently implementing
interfaces between the host and wasm modules.

See for instance the [`uuid`](https://github.com/hotsphink/tritongue/blob/main/modules/uuid/src/lib.rs)
and [`horsejs`](https://github.com/hotsphink/tritongue/blob/main/modules/horsejs/src/lib.rs) modules.

Make sure to install [`cargo-component`](https://github.com/bytecodealliance/cargo-component) first
to be able to build wasm components. We're using a pinned revision of this that can automatically
be installed with `./modules/install-cargo-component.sh` at the moment; we hope to lift that
limitation in the future.

Modules can be hot-reloaded, making it trivial to deploy new modules, or replace existing modules
already running on a server. It is also nice during development iterations on modules. Basically
one can do the following to see changes in close to real-time:

- run tritongue with `cargo run`
- `cd modules/ && cargo watch -x "component build --target=wasm32-unknown-unknown --release"` in another terminal 

The overall generic design is inspired from my previous bot,
[botzilla](https://github.com/bnjbvr/botzilla), that was written in JavaScript and was very
specialized for Mozilla needs.

## Deploying

### Custom Modules

If you want, you can specify a custom modules directory using the `MODULES_PATHS` environment
variable and adding another data volume for it. This can be useful for hacking modules only without
having to compile the host runtime. Here's an example using Docker:

```
docker run -e HOMESERVER="matrix.example.com" \
    -e BOT_USER_ID="@tritongue:example.com" \
    -e BOT_PWD="hunter2" \
    -e ADMIN_USER_ID="@admin:example.com" \
    -e MODULES_PATH="/wasm-modules" \
    -v /host/path/to/data/directory:/opt/tritongue/data \
    -v /host/path/to/modules:/wasm-modules \
    -ti hotsphink/tritongue
```

### Configuration

Tritongue can be configured via config file. The config file can be passed in from the command line:

```bash
cargo run -- config.toml
```

Or it can be placed in `$XDG_CONFIG_HOME`, typically `~/.config/tritongue/config.toml` on XDG
compliant systems. Configuration lives in the document root, for example:

```toml
home_server = "matrix.example.com"
user_id = "@tritongue:example.com"
password = "hunter2"
matrix_store_path = "/path/to/store"
redb_path = "/path/to/redb"
admin_user_id = "@admin:example.com"
modules_path = ["/wasm-modules"]
```

### Module Configuration

It's also possible to pass arbitrary configuration down to specific modules in the config
file. For example:

```toml
[modules_config.pun]
format = "image"
```

This passes the object `{"format": "image"}` to the `pun` module's `init` function. It's
up to specific modules to handle this configuration.

## Is it any good?

[Yes](https://news.ycombinator.com/item?id=3067434).

## Contributing

[![Contributor Covenant](https://img.shields.io/badge/contributor%20covenant-v1.4-ff69b4.svg)](https://www.contributor-covenant.org/version/1/4/code-of-conduct/)

We welcome community contributions to this project.

## Why the name?

Trinity:

This is a *Matrix* bot, coded in Rust and WebAssembly, forming a holy trinity of technologies I (bnjbvr)
love. And, Trinity is also a bad-ass character from the Matrix movie franchise.

Tritongue:

I wanted a new name. I didn't want to spend a lot of time coming up with one. I
plan to add in some Python integration, which led me to think of a forked
tongue, and forked tongue + trinity = tritongue. "Tongue" has a little bit of a
double-meaning of tongue in terms of language, and it's using Rust+WebAssembly+Python.
Also, it's fundamentally a chat bot. But don't overthink it. I certainlyt didn't!

(Trinity is still a better name.)

## License

[LGPLv2 license](LICENSE.md).
