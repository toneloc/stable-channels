# Stable Channels Setup

## 🚀 Quick Start

### 1. Copy Configuration Template
```bash
cp env.example .env
```

### 2. Configure Your Settings
Edit the `.env` file with your values:
```bash
# Required - Get these from your LSP provider
STABLE_CHANNELS_LSP_PUBKEY=your-lsp-pubkey-here
STABLE_CHANNELS_LSP_ADDRESS=your-lsp-address:9737

# Optional - These have sensible defaults
STABLE_CHANNELS_NETWORK=bitcoin
STABLE_CHANNELS_EXPECTED_USD=100.0
```

### 3. Run the Application
```bash
# User interface
cargo run

# LSP backend server
cargo run --bin lsp_backend

# LSP dashboard
cargo run --bin lsp_frontend
```

## ⚙️ Environment Variables

### Required
- `STABLE_CHANNELS_LSP_PUBKEY` - LSP node public key
- `STABLE_CHANNELS_LSP_ADDRESS` - LSP node address (e.g., `100.25.168.115:9737`)

### Optional (with defaults)
- `STABLE_CHANNELS_NETWORK` - Bitcoin network (`bitcoin`/`testnet`)
- `STABLE_CHANNELS_USER_NODE_ALIAS` - User node alias (`user`)
- `STABLE_CHANNELS_USER_PORT` - User node port (`9736`)
- `STABLE_CHANNELS_LSP_NODE_ALIAS` - LSP node alias (`lsp`)
- `STABLE_CHANNELS_LSP_PORT` - LSP node port (`9737`)
- `STABLE_CHANNELS_CHAIN_SOURCE_URL` - Bitcoin API endpoint (`https://blockstream.info/api`)
- `STABLE_CHANNELS_EXPECTED_USD` - Expected USD amount (`100.0`)

### Optional Gateway
- `STABLE_CHANNELS_GATEWAY_PUBKEY` - Gateway node public key
- `STABLE_CHANNELS_GATEWAY_ADDRESS` - Gateway node address

### Optional Bitcoin RPC
- `STABLE_CHANNELS_BITCOIN_RPC_USER` - Bitcoin RPC username
- `STABLE_CHANNELS_BITCOIN_RPC_PASSWORD` - Bitcoin RPC password

## 🛠️ Troubleshooting

- **"Failed to load config"**: Check your `.env` file syntax
- **"LSP_PUBKEY not set"**: Set `STABLE_CHANNELS_LSP_PUBKEY` in your `.env` file
- **"LSP_ADDRESS not set"**: Set `STABLE_CHANNELS_LSP_ADDRESS` in your `.env` file

## 📂 Data Directories

- **User data**: `~/.local/share/StableChannels/user/` (Linux/Mac) or `%APPDATA%\StableChannels\user\` (Windows)
- **LSP data**: `~/.local/share/StableChannels/lsp/` (Linux/Mac) or `%APPDATA%\StableChannels\lsp\` (Windows)