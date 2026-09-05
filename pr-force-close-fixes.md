Title: Fix force-close root causes: cross-process node lock + restore guard

## Why

Two mainnet force closes (Jul 11/13) were traced to iOS app-side LDK state loss — the LSP was exonerated (its node ran with zero restarts across both incidents):

- **FC `0dc926` ("241 vs 240" reestablish)**: the NSE and main app both run LDK nodes against the shared app-group wallet dir, coordinated only by advisory UserDefaults heartbeats. A stale writer regressed channel state by one commitment; the next reestablish force-closed the channel.
- **FC `483ee3` (reset-to-zero reestablish)**: restore-from-mnemonic wipes LDK state by design; reestablishing with empty state against a still-open LSP channel force-closes it silently.

## What

**1. `NodeDirLock` — kernel-enforced flock on `<dataDir>/ldk-node.lock`** (app + NSE copies)
- App acquires before any LDK DB access (node start, gossip restore/extract, network-graph purge) and holds for the node's lifetime including the 30s background grace window; releases after the last post-stop DB write and on all failure paths.
- NSE try-locks; if the app holds the dir it defers the payment (`pending_push_payment`) instead of starting a second node. Gossip strip/VACUUM now only happens under the lock.
- flock auto-releases on process death — a killed process can never wedge the wallet.
- Expire-during-build race handled: `serviceExtensionTimeWillExpire` defers stop/release to the in-flight builder instead of releasing a lock the build still depends on.

**2. Restore guard**
- Derives the node_id from the mnemonic (throwaway build in a temp dir, never started), asks the LSP via new unsigned `POST /api/channel-exists`, and requires an explicit "Restore Anyway" confirmation when a live channel is found. Fails open (with an audit trace) when the LSP is unreachable.

**3. Merged `ios-force-close`**: splice confirmation monitor + idempotency gates — money mutation on splice confirm is now gated on the DB row actually completing (fixes a live double-debit on Android main; iOS equivalent included).

## Verification
- Server: 111/111 tests pass; new handler has unit tests.
- iOS: `StableChannels` and `NotificationService` schemes build clean.
- Adversarial review pass: lock coverage audited (every `ldk_node_data.sqlite` writer is under the lock), lock-lifecycle release matrix verified across all failure paths, expire/build race fixed.

## Deploy notes
- The `/api/channel-exists` route goes live when the front daemon (`stable-channels-lsp`) is rebuilt from this branch/main — safe restart, holds no channels.
- The iOS fixes protect users only once shipped (TestFlight/App Store).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
