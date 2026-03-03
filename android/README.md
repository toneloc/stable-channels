# Stable Channels — Android

Native Android client for Stable Channels, a self-custodial Bitcoin wallet with USD stability via Lightning payment channels.

## Architecture

The app uses Jetpack Compose for UI and [LDK Node](https://lightningdevkit.org/) for Lightning Network operations.

```
app/src/main/java/com/stablechannels/app/
├── MainActivity.kt              # Entry point, notification permission
├── StableChannelsApp.kt         # Application class, Firebase + notification channel init
├── AppState.kt                  # ViewModel — node lifecycle, stability timer, push integration
├── models/                      # Data models (StableChannel, Bitcoin, USD, Trade, etc.)
├── services/
│   ├── NodeService.kt           # LDK Node wrapper (start/stop, payments, channels)
│   ├── PriceService.kt          # BTC price fetching (median of 5 feeds)
│   ├── StabilityService.kt      # Stability calculations (threshold check, balance updates)
│   ├── DatabaseService.kt       # SQLite (channels, trades, payments, price history)
│   ├── TradeService.kt          # Buy/sell trade execution with signed TLV messages
│   └── AuditService.kt          # Audit event logging
├── push/
│   ├── FCMService.kt            # Firebase Cloud Messaging (token + message handling)
│   └── StabilityProcessingService.kt  # Background ForegroundService for push-triggered payments
├── ui/                          # Compose screens (wallet, send, receive, trade, settings)
└── util/
    └── Constants.kt             # App-wide constants (network, peers, thresholds)
```

## Push Notifications (FCM)

When the app is in the background or killed, the LSP sends FCM data messages to trigger stability payments. This mirrors the iOS APNs + Notification Service Extension flow.

**How it works:**
1. On startup, the app registers its FCM token + LDK node ID with the LSP via `POST /api/register-push`
2. The LSP's 180-second stability loop detects price deviations and sends FCM pushes with a `direction` field (`lsp_to_user` or `user_to_lsp`)
3. `FCMService.onMessageReceived()` checks if the main app is active (heartbeat within 10s) — if so, it flags for the main app to handle; otherwise it starts `StabilityProcessingService`
4. `StabilityProcessingService` boots a lightweight LDK node (no RGS gossip, no LSPS2), connects to the LSP, and either receives or sends a stability payment

## Firebase Setup (required for push notifications)

### 1. Create Firebase Project

1. Go to https://console.firebase.google.com
2. Click "Add project" and name it "Stable Channels"
3. Disable Google Analytics (not needed) and click "Create project"

### 2. Add Android App

1. In the Firebase console, click "Add app" and select Android
2. Enter package name: `com.stablechannels.app`
3. Download the generated `google-services.json`
4. Place it at `android/app/google-services.json`

**The `google-services.json` file is required for the Android build to succeed.** It is gitignored since it contains project-specific Firebase config.

### 3. Generate Service Account Key (for LSP server)

1. In Firebase console: Project Settings > Service accounts
2. Click "Generate new private key"
3. Save the JSON file to the LSP server at `data-2/lsp/firebase-service-account.json`

The LSP backend loads this file lazily on first use. If the file is missing, Android pushes are logged but not sent (the LSP still compiles and runs normally).

## Building

### Prerequisites

- Android Studio (Ladybug or newer)
- JDK 17
- Android SDK 35
- `google-services.json` placed at `android/app/google-services.json`

### Build

```bash
cd android
./gradlew assembleDebug
```

The debug APK will be at `app/build/outputs/apk/debug/app-debug.apk`.

### Run

```bash
./gradlew installDebug
```

Or open the `android/` directory in Android Studio and run on a device/emulator.

## Key Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `DEFAULT_LSP_PUBKEY` | `0388948c...` | LSP Lightning node public key |
| `DEFAULT_LSP_ADDRESS` | `100.25.168.115:9737` | LSP Lightning peer address |
| `LSP_PUSH_REGISTER_URL` | `http://100.25.168.115:8080/api/register-push` | Push token registration endpoint |
| `STABILITY_CHECK_INTERVAL_SECS` | 60 | Main app stability check frequency |
| `STABILITY_THRESHOLD_PERCENT` | 0.1% | Minimum deviation to trigger payment |

## Permissions

| Permission | Purpose |
|------------|---------|
| `INTERNET` | Network access for LDK, price feeds, LSP API |
| `ACCESS_NETWORK_STATE` | Check connectivity |
| `CAMERA` | QR code scanning |
| `POST_NOTIFICATIONS` | Show ForegroundService notification (Android 13+) |
| `FOREGROUND_SERVICE` | Run background stability processing |
| `FOREGROUND_SERVICE_DATA_SYNC` | ForegroundService type for data sync |
| `WAKE_LOCK` | Keep CPU awake during push processing |
