# app

React Native turbo-module library wrapping `pm-ffi` — the actual conversation-list/chat/QR-pairing/onboarding UI itself is still M5, not built yet. This is the bridge layer plus a bundled example app that serves as the debug screen for exercising it.

Status: M4 in progress. Scaffolded via `create-react-native-library` (turbo-module, C++/Kotlin/Obj-C native layer, vanilla example app) and wired to `uniffi-bindgen-react-native` (`ubrn`) pointed at the local `../crates/pm-ffi` crate — no git checkout needed, `ubrn.config.yaml` uses the `directory`/`manifestPath` local-crate form.

## What's here

- `src/`, `android/`, `ios/`, `*.podspec` — the turbo-module library scaffold. `android/`, `ios/`, `cpp/` (once generated) and `src/generated/`, `src/Native*`, `src/index.*ts*` are produced by `ubrn` and are not meant to be hand-edited — see `ubrn:clean` below.
- `example/` — the bundled vanilla RN app. This is the "debug screen" M4's exit criteria refers to.
- `ubrn.config.yaml` — points `ubrn` at `pm-ffi`.

## Environment note

This was built and is documented from a Linux (WSL2) machine, which has a hard, unavoidable limit: **iOS builds require Xcode, which only runs on macOS.** There's no way around that from any Linux box, ever.

Beyond that expected limit, Android cross-compilation initially hit a real, unexpected one, since diagnosed and worked around. `rustc`/LLVM (and even plain `cargo`/`rustc` on ordinary host builds) would intermittently segfault under full-parallelism builds. Kernel logs (`dmesg`) traced the actual root cause: this specific WSL2 kernel build (`6.18.33.2-microsoft-standard-WSL2`, confirmed latest via `wsl --update`) has a page-allocator race that corrupts physical page accounting under high concurrent allocation pressure — confirmed directly via `BUG: Bad page state` / `Tainted: [B]=BAD_PAGE` kernel messages showing the *same physical page frame* handed to two different processes at once (`cargo` and `rustc` simultaneously). This is a WSL2 guest-kernel bug, not a Rust, LLVM, or project-code bug, and it scales with the number of vCPUs contending for the allocator at once — this machine's `i7-12700KF` exposes 20 vCPUs to WSL2 (flattened P-core/E-core topology), and the race reproduced reliably under full parallelism but never once under reduced parallelism.

**Workaround (confirmed reliable across 3 clean builds, dev and release):** cap CPU affinity and Cargo's job count before building:

```
taskset -c 0-3 env CARGO_BUILD_JOBS=2 npx ubrn build android --and-generate
```

If this bug resurfaces on a future WSL2 kernel update, the diagnostic path was: check `dmesg` for `BUG: Bad page state` / `Tainted: [B]=BAD_PAGE` entries around the crash timestamp (not just the `segfault` line above them) — that confirms kernel-level corruption rather than an application bug, and the fix is the same parallelism reduction.

What *is* verified end to end: `pm-ffi` compiles and passes its tests on the host target, and the full Android pipeline — `uniffi-bindgen-react-native` config, NDK/SDK toolchain, `cargo-ndk`, cross-compilation for all four ABIs (`arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`), `.a` copy into `jniLibs`, and Kotlin/TS binding generation — completes cleanly in both dev and release profiles.

## Known upstream issue: `uniffi-bindgen-react-native`'s generated CMakeLists.txt

`uniffi-bindgen-react-native@0.31.0-3` (the latest published version) generates an Android `CMakeLists.txt` that resolves its own C++ headers via `node -p "require.resolve('uniffi-bindgen-react-native/package.json')"` at CMake-configure time. That package's own `exports` field doesn't declare `./package.json` as an allowed subpath, so under Node's strict ESM `exports` resolution (confirmed with Node 24) that `require.resolve` call throws `ERR_PACKAGE_PATH_NOT_EXPORTED`, `execute_process` fails, and the resulting CMake variable is silently empty — the actual compile then fails with `fatal error: 'UniffiCallInvoker.h' file not found` (and a giveaway literal `-I/cpp/includes` in the compiler invocation, since the CMake path template collapsed to nothing). This isn't fixable by hand-editing `android/CMakeLists.txt` since `ubrn build android --and-generate` regenerates it every time.

Fixed via `patch-package` (`patches/uniffi-bindgen-react-native+0.31.0-3.patch`), which adds `"./package.json": "./package.json"` to that package's `exports` map so the resolve call succeeds. Applied automatically via `npm install`'s `postinstall` hook — no manual step needed. Worth re-checking if `uniffi-bindgen-react-native` ships a version past `0.31.0-3` (the patch can likely be dropped then).

## Setup

```
npm install
taskset -c 0-3 env CARGO_BUILD_JOBS=2 npx ubrn build android --and-generate   # requires ANDROID_NDK_HOME set (see below)
```

Add `--release` for a release build (also verified working under the same mitigation).

The `taskset`/`CARGO_BUILD_JOBS` prefix works around a WSL2 kernel bug (see above) — on a native Linux machine or a fixed WSL2 kernel, a plain `npx ubrn build android --and-generate` should work.

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
