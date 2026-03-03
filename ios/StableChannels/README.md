# Stable Channels — iOS

Native iOS client for Stable Channels, a self-custodial Bitcoin wallet with USD stability via Lightning payment channels.

## Architecture

The app uses SwiftUI for UI and [LDK Node](https://lightningdevkit.org/) for Lightning Network operations. It consists of two targets: the main app and a Notification Service Extension (NSE) that processes stability payments in the background.

```
StableChannels/                          # Main app target
├── StableChannelsApp.swift              # App entry point + AppDelegate (APNs registration)
├── AppState.swift                       # Main state management + LDK lifecycle
├── Models/
│   ├── StableChannel.swift              # Core types (Bitcoin, USD, StableChannel)
│   └── Trade.swift                      # Trade & database record types
├── Services/
│   ├── NodeService.swift                # LDK Node wrapper (start/stop, payments, channels)
│   ├── PriceService.swift               # Multi-feed BTC price fetching (median of 5 sources)
│   ├── StabilityService.swift           # Stability calculations (threshold check, balance updates)
│   ├── DatabaseService.swift            # SQLite (channels, trades, payments, price history)
│   ├── TradeService.swift               # Buy/sell trade execution with signed TLV messages
│   └── AuditService.swift               # Audit event logging
├── Utilities/
│   ├── Constants.swift                  # Network config, peers, thresholds, app group ID
│   └── Extensions.swift                 # Helper extensions
├── Views/
│   ├── ContentView.swift                # Root view
│   ├── MainTabView.swift                # Tab navigation
│   ├── Home/                            # Balance display, fund wallet
│   ├── Trade/                           # Buy/sell BTC screens
│   ├── Transfer/                        # Send, receive, on-chain screens
│   ├── History/                         # Payment and trade history
│   └── Settings/                        # Settings screen
├── Info.plist                           # Background modes, camera usage, ATS config
├── StableChannels.entitlements          # Push notifications + app group
└── Assets.xcassets/                     # App icon

NotificationService/                     # Notification Service Extension target
├── NotificationService.swift            # Background LDK node for stability payments
├── Info.plist                           # Extension point identifier
└── NotificationService.entitlements     # App group (shared container access)

StableChannelsTests/
└── StabilityServiceTests.swift          # Unit tests
```

## Push Notifications (APNs)

When the app is in the background or killed, the LSP sends APNs push notifications to trigger stability payments. The Notification Service Extension (NSE) intercepts these pushes and boots a lightweight LDK node to process payments without the user opening the app.

### Flow: App Killed (NSE handles payment)

1. LSP's 180-second stability loop detects a price deviation for a user's channel
2. LSP sends APNs push with `mutable-content: 1` and payload `{"stability": {"direction": "lsp_to_user" | "user_to_lsp"}}`
3. iOS wakes the NSE (~30 seconds of execution time)
4. NSE checks main app heartbeat — if active within 10s, flags `pending_push_payment` and exits
5. Otherwise, NSE builds a lightweight LDK node (no RGS gossip, relaxed sync intervals) from the shared app group container
6. NSE connects to LSP peer
7. **`lsp_to_user`** (price dropped, LSP sends us sats): polls `node.nextEvent()` for up to 22 seconds waiting for `PaymentReceived`
8. **`user_to_lsp`** (price rose, user owes LSP): reads channel state from SQLite (C API), fetches median BTC price from 3 sources, calculates stability payment, sends keysend with TLV marker
9. Updates notification content with result ("Stability Payment Received" / "Stability Payment Sent") or falls back to "Payment Pending"
10. If `serviceExtensionTimeWillExpire()` fires, flags `pending_push_payment` for main app to retry on next launch

### Flow: App Backgrounded (AppDelegate handles)

1. Push arrives with `content-available: 1`
2. `AppDelegate.didReceiveRemoteNotification` posts to `NotificationCenter`
3. `AppState` reconnects to LSP and refreshes balances
4. Completion handler waits 25 seconds for payment to settle

### Flow: App in Foreground

1. Push arrives, `userNotificationCenter(willPresent:)` shows banner + sound
2. Main app stability timer handles payment processing normally

### NSE LDK Node Configuration

The NSE boots a stripped-down LDK node optimized for fast startup and minimal memory:

- Same seed file and storage directory as main app (shared via app group)
- `trustedPeers0conf` = LSP pubkey (skip channel confirmation wait)
- Relaxed sync intervals: 600s on-chain, 600s Lightning, 3600s fee rate
- No RGS gossip sync (not needed — route to LSP is already known)
- No LSPS2 (not opening channels in the background)
- Connects only to LSP peer

### Data Sharing Between App and NSE

Both the main app and NSE access the same data through the App Group container:

**App Group:** `group.com.stablechannels.app`

**Shared file container:**
```
{AppGroupContainer}/StableChannels/user/
├── keys_seed              # LDK node seed (binary)
├── ldk_node.log           # LDK debug log
├── audit_log.txt          # App audit events
├── stablechannels.db      # SQLite: channels, trades, payments, price history
├── ldk_node_data.sqlite   # LDK internal data
└── nse_debug.log          # NSE-specific debug log
```

**Shared UserDefaults** (`UserDefaults(suiteName: "group.com.stablechannels.app")`):

| Key | Writer | Purpose |
|-----|--------|---------|
| `main_app_last_active` | Main app | Heartbeat timestamp — NSE skips if < 10s ago |
| `nse_processing` | NSE | Flag indicating NSE is currently running |
| `nse_last_active` | NSE | NSE heartbeat (2-second interval) |
| `pending_push_payment` | NSE | Flag for main app to retry on next launch |
| `node_id` | Main app | LDK node ID for push token registration |

### APNs Setup

**Entitlements** (`StableChannels.entitlements`):
- `aps-environment`: `development` (change to `production` for App Store)
- `com.apple.security.application-groups`: `group.com.stablechannels.app`

**NSE Entitlements** (`NotificationService.entitlements`):
- `com.apple.security.application-groups`: `group.com.stablechannels.app`

**Background Modes** (Info.plist):
- `fetch` — background refresh
- `processing` — background processing
- `remote-notification` — push notification wake

**APNs Key:**
The LSP server uses an APNs authentication key (`.p8` file) to send pushes. This is configured in the LSP backend with the Key ID, Team ID, and bundle topic.

## Building

### Prerequisites

- Xcode 16.0+
- iOS 17.0+ deployment target
- Apple Developer account with Push Notifications capability
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) (for regenerating the Xcode project)

### Open in Xcode

```bash
cd ios/StableChannels
open StableChannels.xcodeproj
```

Select a device or simulator and build (Cmd+B) / run (Cmd+R).

### Regenerate Xcode Project

If you modify `project.yml` (targets, settings, dependencies):

```bash
cd ios/StableChannels
xcodegen generate
```

Then open the regenerated `.xcodeproj`.

### Resolve Package Dependencies

After cloning or after a DerivedData wipe:

Xcode menu: File > Packages > Resolve Package Versions

### Troubleshooting: "Executable not codesigned"

If Xcode fails with a code signing error after switching branches or updating:

1. Quit Xcode completely
2. `rm -rf ~/Library/Developer/Xcode/DerivedData/StableChannels-*`
3. Reopen the project in Xcode
4. File > Packages > Resolve Package Versions
5. Build

Xcode caches stale build artifacts and won't rebuild properly if DerivedData is only deleted while Xcode is open.

## Dependencies

Managed via Swift Package Manager (defined in `project.yml`):

| Package | Version | Purpose |
|---------|---------|---------|
| [LDK Node](https://github.com/lightningdevkit/ldk-node) | 0.7.0 (exact) | Lightning Network node |
| [KeychainAccess](https://github.com/kishikawakatsumi/KeychainAccess) | 4.2.2+ | Secure key storage |
| [CodeScanner](https://github.com/twostraws/CodeScanner) | 2.5.0+ | QR code scanning |

The NSE target depends only on LDK Node (no KeychainAccess or CodeScanner needed).

## Key Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `defaultLSPPubkey` | `0388948c...` | LSP Lightning node public key |
| `defaultLSPAddress` | `100.25.168.115:9737` | LSP Lightning peer address |
| `appGroupIdentifier` | `group.com.stablechannels.app` | Shared container for app + NSE |
| `stabilityCheckIntervalSecs` | 60 | Main app stability check frequency |
| `stabilityThresholdPercent` | 0.1% | Minimum deviation to trigger payment |
| `stabilityPaymentCooldownSecs` | 120 | Minimum time between stability payments |
| `stableChannelTLVType` | 13377331 | Custom TLV type for stability markers |

## Testing Push Notifications

### Simulator (drag-and-drop .apns files)

Drag `test_push.apns` or `test_push_silent.apns` onto the iOS Simulator to trigger a test push.

**test_push.apns** (visible alert + NSE trigger):
```json
{
    "Simulator Target Bundle": "com.stablechannels.app",
    "aps": {
        "alert": { "title": "Stability", "body": "Processing stability payment..." },
        "mutable-content": 1,
        "sound": "default"
    },
    "stability": { "direction": "lsp_to_user" }
}
```

**test_push_silent.apns** (background wake only):
```json
{
    "Simulator Target Bundle": "com.stablechannels.app",
    "aps": { "content-available": 1 }
}
```

### Device (send_push.sh)

Send a real APNs push to a physical device:

```bash
./send_push.sh lsp_to_user
```

Requires `AuthKey_*.p8`, device token, and correct Team ID / Key ID configured in the script.

## Project Configuration (project.yml)

The Xcode project is generated from `project.yml` using XcodeGen. Key settings:

| Setting | Value |
|---------|-------|
| Bundle ID (app) | `com.stablechannels.app` |
| Bundle ID (NSE) | `com.stablechannels.app.NotificationService` |
| Deployment target | iOS 17.0 |
| Swift version | 5.9 |
| Development team | VJF3VBKXV9 |
| Code signing | Automatic |
