[![main on CLN v24.02.2](https://github.com/toneloc/stable-channels/actions/workflows/main_v24.02.yml/badge.svg?branch=main)](https://github.com/toneloc/stable-channels/actions/workflows/main_v24.02.yml)

## Stable Channels

The Stable Channels Wallet lets users peg a portion of their bitcoin to a dollar balance, all while staying self-custodial and 100% in bitcoin. 

<p align="center">
  <img width="337" height="524" src="https://github.com/user-attachments/assets/c9a15617-dc60-48a6-a21f-18f7c2d3f95b" />
  &#x2003;&#x2003;&#x2003;&#x2003; <!-- 4 em-spaces (~wide gap) -->
  <img width="331" height="520" src="https://github.com/user-attachments/assets/43191915-eaad-4593-bb85-3af40be15d01" />
</p>

### Downloads
- [macOS](https://github.com/toneloc/stable-channels/releases/download/0.6/stable-channels-mac.zip)
- [Windows](https://github.com/toneloc/stable-channels/releases/download/0.6/stable-channels-windows.zip)
- [Linux](https://github.com/toneloc/stable-channels/releases/download/0.6/stable-channels-linux.zip)

Check out the latest releases in [Releases](https://github.com/toneloc/stable-channels/releases)

### Overview

The Stable Channels Desktop Wallet runs a full Lightning node that lets users stabilize the money that they send to themselves during onboarding. 

The LSP/Server keeps the user stable by overcollateralizing the Stable Channel by 100% at the time of channel creation. Each user (the stability receiver and the LSP/stability provider) puts in the same amount of bitcoin, and the stability mechanism is activated. 

The stability mechanism works like this: each node queries five exchange price feeds every minute. Based on the updated price, they adjust the channel balance with their counterparty to keep the stability receiver's balance at a fixed dollar value (e.g., $100,000 of bitcoin).

<p align="center">
  <img src="./sc.gif" alt="Stable Channels Architecture" width="700"/>
</p>

Both parties remain self-custodial and can opt out anytime via cooperative or forced on-chain channel closure. 

The project is in-progress and is built on LDK Node and Rust. Prior versions were compatible with LND and CLN. These legacy implementations can be found in `/legacy`. 

Links with examples:
- **Basic example:** [Twitter thread](https://x.com/tonklaus/status/1729567459579945017)
- **In-depth discussion:** [Delving Bitcoin](https://delvingbitcoin.org/t/stable-channels-peer-to-peer-dollar-balances-on-lightning)
- **Podcast discussion:** [Stephan Livera Podcast — Episode 591](https://stephanlivera.com/episode/591/)
- **Project website:** [StableChannels.com](https://www.stablechannels.com)

### Run the User Desktop Wallet 

You can check out the latest builds for macOS, Windows, or Linux here - https://github.com/toneloc/stable-channels/releases

### Run from Source

To run the app from this source code, please install Rust on your OS.

Using a fresh Ubuntu or on Windows? You may need to install OpenSSL libraries or Perl. 

Linux - `sudo apt-get install -y pkg-config libssl-dev` and `curl`.

Windows - `winget install StrawberryPerl.StrawberryPerl`. Windows also requires a few other things as well.

Clone the repo `git clone https://github.com/toneloc/stable-channels` and `cd` into it.

Run `cargo run --bin stable-channels user`. This will start the app on mainnet. Pay the invoice and you are good to go.

Logs and key files can be found in these directories:
- Linux   - `~/.local/share/StableChannels`
- Mac     - `~/Library/Application\ Support/StableChannels`
- Windows - `%APPDATA%\StableChannels`
<sub><sup>*actual directory might differ depending on user's system</sup></sub>

### Configuration

The application works out of the box with smart defaults, but supports customization via environment variables or a `.env` file:

#### Quick Setup (Works Out of the Box)
```bash
# Just run - no configuration needed!
cargo run --bin stable-channels user
```

#### Custom Configuration (Optional)
```bash
# Copy configuration template
cp env.example .env

# Edit with your values
nano .env / vi .env

# Run the application
cargo run --bin stable-channels user
```

#### Environment Variables (Optional - has smart defaults)
- `STABLE_CHANNELS_LSP_PUBKEY` - LSP node public key (default: included)
- `STABLE_CHANNELS_LSP_ADDRESS` - LSP node address (default: `100.25.168.115:9737`)
- `STABLE_CHANNELS_NETWORK` - Bitcoin network (`bitcoin`/`signet`)
- `STABLE_CHANNELS_USER_NODE_ALIAS` - User node alias (`user`)
- `STABLE_CHANNELS_USER_PORT` - User node port (`9736`)
- `STABLE_CHANNELS_LSP_NODE_ALIAS` - LSP node alias (`lsp`)
- `STABLE_CHANNELS_LSP_PORT` - LSP node port (`9737`)
- `STABLE_CHANNELS_CHAIN_SOURCE_URL` - Bitcoin API endpoint (`https://blockstream.info/api`)
- `STABLE_CHANNELS_EXPECTED_USD` - Expected USD amount (`100.0`)

#### Running Different Components
```bash
# User interface
cargo run --bin stable-channels user

# LSP backend server
cargo run --bin lsp_backend

# LSP dashboard
cargo run --bin lsp_frontend
```

More instructions on running the LSP backend are forthcoming.

### Stable Channels Process

Every 30 seconds, the price of bitcoin:

- **(a) Goes up:**
  - **User/Stable Receiver loses bitcoin.**
    - Less bitcoin is needed to maintain the dollar value.
    - The User/Stable Receiver pays the LSP/Stable Provider.
  
- **(b) Goes down:**
  - **User/Stable Receiver gains bitcoin.**
    - More bitcoin is needed to maintain the dollar value.
    - The LSP/Stable Provider pays the User/Stable Receiver.
  
- **(c) Stays the same:**
  - **No action required.**

*Note: Stable Channels are currently non-routing channels. Work is ongoing to add routing and payment capabilities.*

## Payout Examples (entry = $100,000/BTC)

Each side puts in 1 BTC at $100,000.

Abbreviations:
- SR = Stable Receiver (targeting $100,000)
- SP = Stable Provider
- Δ = Delta / Change

| Price Change (%) | New BTC Price | SR (BTC) | SR (USD) | SP (BTC) | SP (USD) | SR Fiat Δ$ | SR BTC Δ | SR Fiat Δ% | SR BTC Δ% | SP Fiat Δ$ | SP BTC Δ | SP Fiat Δ% | SP BTC Δ% |
|------------------|---------------|----------|----------|----------|----------|------------|----------|------------|-----------|------------|----------|------------|-----------|
| -30              | 70,000.00     | 1.43     | 100,000.00| 0.57    | 40,000.00| 0.00       | +0.43    | 0%         | +42.86%   | -60,000.00 | -0.43    | -60.00%    | -42.86%   |
| -20              | 80,000.00     | 1.25     | 100,000.00| 0.75    | 60,000.00| 0.00       | +0.25    | 0%         | +25.00%   | -40,000.00 | -0.25    | -40.00%    | -25.00%   |
| -10              | 90,000.00     | 1.11     | 100,000.00| 0.89    | 80,000.00| 0.00       | +0.11    | 0%         | +11.11%   | -20,000.00 | -0.11    | -20.00%    | -11.11%   |
| 0                | 100,000.00    | 1.00     | 100,000.00| 1.00    | 100,000.00| 0.00      | 0.00     | 0%         | 0%        | 0.00       | 0.00     | 0%         | 0%        |
| 10               | 110,000.00    | 0.91     | 100,000.00| 1.09    | 120,000.00| 0.00      | -0.09    | 0%         | -9.09%    | +20,000.00 | +0.09    | +20.00%    | +9.09%    |
| 20               | 120,000.00    | 0.83     | 100,000.00| 1.17    | 140,000.00| 0.00      | -0.17    | 0%         | -16.67%   | +40,000.00 | +0.17    | +40.00%    | +16.67%   |
| 30               | 130,000.00    | 0.77     | 100,000.00| 1.23    | 160,000.00| 0.00      | -0.23    | 0%         | -23.08%   | +60,000.00 | +0.23    | +60.00%    | +23.08%   |

### Acknowledgements

Thanks to countless open-source bitcoin developers and organizations who have made this work possible. Satoshi Lives!

Thanks to all of the Lightning Network core developers, and all of the bitcoin open-source devs on whose giant shoulders we stand. 
