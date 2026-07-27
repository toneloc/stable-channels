# Contributing to Stable Channels

Thanks for helping build self-custodial dollar stability on Lightning. Small, focused PRs are the fastest path to a merge.

## One-time setup

Install the git hooks after cloning:

```bash
./.githooks/install.sh
```

This runs secret scanning, `cargo fmt`, `cargo clippy`, and a large-file guard on every commit.

## Building

**Desktop (Rust, macOS/Windows/Linux):**

```bash
cargo run --bin stable-channels user   # runs the wallet on mainnet
cargo test                             # unit tests
```

Linux needs `pkg-config libssl-dev`; Windows needs Strawberry Perl (see README "Technical tips").

**iOS:**

Open `ios/StableChannels/StableChannels.xcodeproj` in Xcode and build. Dependencies (including the LDK Node bindings) resolve via Swift Package Manager.

**Android:**

```bash
cd android && ./gradlew assembleDebug
```

Requires JDK 17 and the Android SDK. The LDK Node AAR is pulled from a GitHub release via the Ivy repo in `settings.gradle.kts`. Push notifications need a `google-services.json` (not required to build).

## Pull request guidelines

- Keep PRs small and single-purpose; describe the user-visible behavior change, not just the code change.
- Say which platforms the change affects. The three apps (desktop, iOS, Android) aim for behavioral parity — if you fix a bug on one platform, note whether it exists on the others so it can be ported.
- This is money software: changes touching the stability engine, channel lifecycle, splicing, or payment reconciliation get extra scrutiny. Include the reasoning, not just the diff, and add tests where the logic is pure enough to test.
- `cargo fmt` and `cargo clippy` must pass (the pre-commit hooks enforce this for the Rust core).
- Never commit secrets, seeds, keystores, or node data. The secret-scanning hook is a backstop, not permission to be careless.

## Reporting issues

Use GitHub Issues for bugs and feature requests. For anything security-sensitive (potential fund loss, key handling, protocol edge cases), email tony@stablechannels.com privately instead of opening a public issue.

## License

By contributing, you agree that your contributions are licensed under the [GPLv3](./COPYING).
