# iOS vs Android — user-facing text differences

Derived from the E2E flows (`e2e/flows/*.yaml`), which encode empirically
verified copy per platform (every `(Android|iOS)` regex alternation and
`platform:` conditional marks a real divergence seen on-device). Covers only
the screens the canonical flows touch (01–09, 12); 10/11 not yet automated.

Many iOS strings come from the localization catalog (`Localizable.xcstrings`),
which **overrides** the Swift code's `defaultValue` — so the shipped iOS copy
can differ from what the source suggests.

| # | Screen / moment | Flow | Android | iOS |
|---|-----------------|------|---------|-----|
| 1 | First-launch notifications prompt | 01 | *(none)* | system "Allow" dialog |
| 2 | JIT / first-payment notice | 01 | "First payment — a channel will be opened automatically via LSP" | "Make your first payment to activate the channel" |
| 3 | Receive — invoice control | 01, 04 | **"Copy"** (button) | **"Your Lightning Invoice"** (label) |
| 4 | Receive — dismiss sheet | 01, 04 | **"Cancel"** | **"Done"** |
| 5 | Payment-received banner | 01, 04 | **"Payment received"** | **"Received"** |
| 6 | BTC→USD amount prompt | 02 | **"Amount (USD)"** | **"How much BTC to convert to USD?"** |
| 7 | Trade confirm — title | 02 | **"Confirm Order"** | **"Confirm BTC → USD"** |
| 8 | Trade confirm — button | 02, 08 | **"Confirm Order"** | direction label **"BTC → USD"** / **"USD → BTC"** |
| 9 | Onchain-receive back nav | 05 | **"Back"** | **"Receive"** (nav-back labeled by prev screen) |
| 10 | Move onchain funds → channel | 05 | **"Swap"** | **"Move"** |
| 11 | Send — address field | 06, 07, 12 | **"Lightning invoice or Onchain address"** (label) | **"lnbc1…"** (placeholder) |
| 12 | Send — primary button | 06 | **"Send"** | **"Send"** (xcstrings overrides code default "Send Payment") |
| 13 | Lightning-send success | 06 | **"Done"** screen | **"Payment confirmed"** capsule (auto-dismisses to Home) |
| 14 | Onchain-send amount chip | 07 | **"Amount (USD)"** | **"Onchain Address"** |
| 15 | USD→BTC amount prompt | 08 | **"Amount (USD)"** | **"How much USD to convert to BTC?"** |
| 16 | USD→BTC confirm — title | 08 | **"Confirm Order"** | **"Confirm USD → BTC"** |
| 17 | Close-channel alert — confirm | 09 | **"Close"** | **"Close Channel"** (localized) |

## Not just text — behaviour / feature gaps surfaced by the tests

- **Send Max (offboard, flow 12):** Android has a **"Send Max"** sweep button;
  iOS has **no send-max affordance** — the flow enters a fixed amount instead.
  This is a real feature gap, not a copy difference.
- **Lightning-send completion (flow 06):** Android shows a persistent **Done**
  screen; iOS auto-dismisses to Home with a transient "Payment confirmed"
  capsule (can race away before assertion).
- **Onchain sync cadence (flow 05):** iOS onchain wallet sync interval is ~120s
  vs Android's faster tick — iOS flows need longer waits for deposits/splices.

## Consistency candidates (if you want to converge the copy)

- "Confirm Order" (Android) vs "Confirm <direction>" (iOS) — pick one.
- "Amount (USD)" (Android) vs the long "How much … to convert …?" question
  (iOS) — the trade amount prompt differs on every trade screen.
- "Payment received" vs "Received"; "Copy" vs "Your Lightning Invoice";
  "Swap" vs "Move" — small label mismatches across the same feature.
- Close-channel confirm: "Close" vs "Close Channel".
