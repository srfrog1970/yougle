# app

React Native turbo-module library wrapping `pm-ffi` — the actual conversation-list/chat/QR-pairing/onboarding UI itself is still M5, not built yet. This is the bridge layer plus a bundled example app that serves as the debug screen for exercising it.

Status: M4 in progress. Scaffolded via `create-react-native-library` (turbo-module, C++/Kotlin/Obj-C native layer, vanilla example app) and wired to `uniffi-bindgen-react-native` (`ubrn`) pointed at the local `../crates/pm-ffi` crate — no git checkout needed, `ubrn.config.yaml` uses the `directory`/`manifestPath` local-crate form.

## What's here

- `src/`, `android/`, `ios/`, `*.podspec` — the turbo-module library scaffold. `android/`, `ios/`, `cpp/` (once generated) and `src/generated/`, `src/Native*`, `src/index.*ts*` are produced by `ubrn` and are not meant to be hand-edited — see `ubrn:clean` below.
- `example/` — the bundled vanilla RN app. This is the "debug screen" M4's exit criteria refers to.
- `ubrn.config.yaml` — points `ubrn` at `pm-ffi`.

## Environment note

This was built and is documented from a Linux (WSL2) machine, which has a hard, unavoidable limit: **iOS builds require Xcode, which only runs on macOS.** There's no way around that from any Linux box, ever.

Beyond that expected limit, Android cross-compilation hit a real, unexpected one: `rustc` itself segfaults (SIGSEGV inside its LLVM backend) partway through compiling the `aarch64-linux-android` target — reproducibly, on ordinary, widely-used crates (`serde`, `zeroize`, `xml-rs`, and others, different ones each run), even fully serial (`CARGO_BUILD_JOBS=1`) with the stack size increased well past what `rustc` itself suggested (up to 128 MB). Host (`x86_64-unknown-linux-gnu`) builds have been completely stable across this entire project — dozens of clean `cargo build`/`cargo test` runs, zero crashes — so this looks specific to the aarch64-Android cross-compilation codegen path in this particular WSL2 environment, not a bug in this project's code. Likely worth trying on a native Linux machine or inside a proper Linux VM (as opposed to WSL2) before assuming it's something to fix here.

What *is* verified end to end: `pm-ffi` itself compiles cleanly and passes its tests on the host target (confirming the uniffi interface is correct), and the whole pipeline up to native compilation — `uniffi-bindgen-react-native` config, Android NDK/SDK toolchain install, `cargo-ndk` invocation — runs and reaches `rustc` successfully. The break is specifically in `rustc`'s own aarch64 codegen, past the point where anything in this repo could be at fault.

## Setup

```
npm install
npx ubrn build android --and-generate   # requires ANDROID_NDK_HOME set (see below)
```

Requires, beyond what `pm-store` already needs (`libssl-dev`, `pkg-config`):
- `cargo install cargo-ndk`
- `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android`
- A Java runtime and the Android NDK, with `ANDROID_NDK_HOME` set to the NDK's root directory.
- `pm-store`'s `rusqlite` dependency uses the `bundled-sqlcipher-vendored-openssl` feature (not plain `bundled-sqlcipher`) specifically so cross-compiling doesn't need a pre-built target OpenSSL lying around — it builds OpenSSL from source for whatever target is active.

For iOS (macOS only): `npx ubrn build ios --and-generate`.

## Cleaning generated files

```
npm run ubrn:clean
```
