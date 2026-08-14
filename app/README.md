# app

React Native turbo-module library wrapping `pm-ffi`, plus the actual app UI (`example/`) — onboarding, conversation list, chat, QR/paste-code pairing, mailbox management, recovery-phrase view, and backup export/import, all wired to the real Rust core.

Status: M6 complete. Scaffolded via `create-react-native-library` (turbo-module, C++/Kotlin/Obj-C native layer) and wired to `uniffi-bindgen-react-native` (`ubrn`) pointed at the local `../crates/pm-ffi` crate — no git checkout needed, `ubrn.config.yaml` uses the `directory`/`manifestPath` local-crate form. Verified by actually running the app end to end on two simultaneous headless Android emulators (real identity creation, real SQLCipher DB, real Keychain, real mutual QR/paste-code pairing between two independent devices, real Local-to-local direct P2P delivery and "delivered" receipts both directions, real Server-mailbox round trip against a live `pm-node`, real backup export to the OS share sheet) — not just a build check.

## What's here

- `src/`, `android/`, `ios/`, `*.podspec` — the turbo-module library scaffold. `android/`, `ios/`, `cpp/` (once generated) and `src/generated/`, `src/Native*`, `src/index.*ts*` are produced by `ubrn` and are not meant to be hand-edited — see `ubrn:clean` below.
- `example/` — the actual app. `example/src/App.tsx` is the entry point; `example/src/screens/` has one file per PRD §7 screen, `example/src/lib/client.tsx` holds the `FfiClient` React context.
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

## Other issues that only show up at runtime, not at build time

Found running the real app on-device, not from `cargo test`/`tsc`/`gradle build` — worth knowing about if this stack is touched again:

- **`uniffi` version must exactly match `uniffi-bindgen-react-native`'s own pin.** `pm-ffi`'s `Cargo.toml` pins `uniffi = "=0.31.0"` to match `uniffi-bindgen-react-native`'s own `Cargo.toml`. A mismatch compiles and tests fine on both sides (the JS and Rust each just embed their own scaffolding-version number) and only fails the moment the app actually calls in, as `Incompatible versions of uniffi were used to build the JS (N) from the Rust (M)`.
- **Every `pm-ffi` async method body is wrapped in `async_compat::Compat`** (see `compat()` in `crates/pm-ffi/src/lib.rs`). uniffi's async bridge polls exported futures from whatever thread the foreign (Kotlin/JS) side drives them from, not necessarily one with a Tokio runtime entered — `iroh` needs one. `#[tokio::test]` always provides one, so this is invisible in Rust-only testing and fails at runtime as `there is no reactor running, must be called from the context of a Tokio 1.x runtime`.
- **`example/metro.config.js` needs `watchFolders`/`resolver.extraNodeModules`** pointing `yougle-native` at `..` — Metro has no built-in notion of a local, unpublished sibling package, so without this every screen fails at launch with `Cannot find module 'yougle-native'`.
- **`app/package.json` needs a `"react-native-builder-bob": { "source": "src" }` block** — without it, `example/babel.config.js`'s call into `react-native-builder-bob/babel-config` throws `Couldn't determine the source directory` and the JS bundle never transforms.
- **The debug APK must be built for the ABI the emulator actually runs.** `gradlew assembleDebug -PreactNativeArchitectures=arm64-v8a` on an `x86_64` AVD installs fine but crashes on launch (`SoLoaderDSONotFoundError: couldn't find DSO to load: libreactnative.so`) since only arm64 native libs got packaged. Match the flag to the AVD's ABI (`ubrn build android` itself always cross-compiles all four ABIs regardless of this flag — it only controls what Gradle packages into the APK).
- **Hermes ships `TextEncoder` but not `TextDecoder`.** `example/src/lib/bytes.ts`'s `bufferToText` used to call `new TextDecoder().decode(...)`, which compiles and type-checks fine but throws `Property 'TextDecoder' doesn't exist` the moment any message actually renders — invisible until M6's live two-device test was the first thing to actually display message content in the running app. Fixed with a manual UTF-8 decoder (no global dependency).
- **A `multiline` `TextInput` can crash rendering a long unbroken pasted string.** M6 lengthened the pairing code (the added `transportKey` field) enough to reliably trigger a Fabric/Android bug — `Exception in HostFunction: java.lang.IllegalStateException: Required value was null` in `TextLayoutManager.getOrCreateSpannableForText` — when pasting the ~550-character real code into the Pairing screen's "Enter code" field. Reproduced independent of how the text was entered (paste, typed, chunked); a plain read-only `Text` showing the same string was unaffected, only the editable `TextInput`. Fixed with `textBreakStrategy="simple"` on that `TextInput` (the documented workaround for this class of bug); short strings never triggered it.

## Headless emulator verification (WSL2)

No physical device or GUI needed. `/dev/kvm` is present in WSL2 but the invoking user isn't in the `kvm` group by default — `sudo gpasswd -a $USER kvm` then either `sg kvm -c '<command>'` (no restart needed) or a fresh login:

```
sdkmanager "emulator" "system-images;android-34;google_apis;x86_64"
avdmanager create avd -n yougle -k "system-images;android-34;google_apis;x86_64" -d pixel_6
sg kvm -c "emulator -avd yougle -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect &"
adb wait-for-device shell 'while [[ -z $(getprop sys.boot_completed) ]]; do sleep 1; done'

adb install -r example/android/app/build/outputs/apk/debug/app-debug.apk
adb reverse tcp:8081 tcp:8081        # example app is a debug build; needs Metro
npx react-native start               # from example/, separately
adb shell am start -n youglenative.example/.MainActivity
adb exec-out screencap -p > screen.png   # visual inspection
adb logcat -d | grep -iE "ReactNativeJS|FATAL"
```

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

## Release signing

`example/android/app/build.gradle`'s `release` build type signs with
whatever `example/android/keystore.properties` (gitignored, not
committed) points at, falling back to the debug keystore only when that
file is absent. The actual release keystore used to sign published APKs
lives outside the repo entirely, at `~/.yougle-release/` on the
machine that built the current release — regenerate a new one anytime
with `keytool -genkeypair`, matching the shape in
`example/android/keystore.properties.example`. Note that a new keystore
produces a different signing key, so devices with an existing install
signed by the old key will need to uninstall before installing a build
signed by the new one (Android refuses same-package updates across a
signature change).
