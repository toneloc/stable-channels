#!/bin/bash
set -e

SERVER="ubuntu@100.25.168.115"
SSH_KEY="$HOME/.ssh/full_node.pem"
REMOTE_DIR="/home/ldk/stable-channels"

echo "=== LSP Deploy ==="

# 1. SCP source files
echo "[1/3] Uploading source files..."
scp -i "$SSH_KEY" Cargo.toml "$SERVER:$REMOTE_DIR/Cargo.toml"
scp -i "$SSH_KEY" src/lib.rs "$SERVER:$REMOTE_DIR/src/lib.rs"
scp -i "$SSH_KEY" src/stable.rs "$SERVER:$REMOTE_DIR/src/stable.rs"
scp -i "$SSH_KEY" src/types.rs "$SERVER:$REMOTE_DIR/src/types.rs"
scp -i "$SSH_KEY" src/constants.rs "$SERVER:$REMOTE_DIR/src/constants.rs"
scp -i "$SSH_KEY" src/db.rs "$SERVER:$REMOTE_DIR/src/db.rs"
scp -i "$SSH_KEY" src/price_feeds.rs "$SERVER:$REMOTE_DIR/src/price_feeds.rs"
scp -i "$SSH_KEY" src/audit.rs "$SERVER:$REMOTE_DIR/src/audit.rs"
scp -i "$SSH_KEY" src/historical_prices.rs "$SERVER:$REMOTE_DIR/src/historical_prices.rs"
scp -i "$SSH_KEY" src/bin/lsp_backend.rs "$SERVER:$REMOTE_DIR/src/bin/lsp_backend.rs"

# 2. Build on server
echo "[2/3] Building on server..."
ssh -i "$SSH_KEY" "$SERVER" "cd $REMOTE_DIR && source ~/.cargo/env && cargo build --release --bin lsp_backend"

# 3. Stop, sleep, start service
echo "[3/4] Restarting lsp.service..."
ssh -i "$SSH_KEY" "$SERVER" "sudo systemctl stop lsp.service && sleep 3 && sudo systemctl start lsp.service"

# 4. Health check — wait for LDK node to fully start
echo "[4/4] Waiting for LDK node startup..."
for i in $(seq 1 30); do
    if ssh -i "$SSH_KEY" "$SERVER" "sudo journalctl -u lsp.service --since '1 minute ago' --no-pager -q" 2>/dev/null | grep -q '\[Init\] Node ID:'; then
        NODE_LINE=$(ssh -i "$SSH_KEY" "$SERVER" "sudo journalctl -u lsp.service --since '1 minute ago' --no-pager -q" 2>/dev/null | grep '\[Init\] Node ID:')
        echo "OK: $NODE_LINE"
        echo "=== Done ==="
        exit 0
    fi
    # Check if service crashed
    if ! ssh -i "$SSH_KEY" "$SERVER" "systemctl is-active --quiet lsp.service" 2>/dev/null; then
        echo "FAIL: lsp.service is not running!"
        ssh -i "$SSH_KEY" "$SERVER" "sudo journalctl -u lsp.service --since '2 minutes ago' --no-pager -n 20"
        exit 1
    fi
    sleep 2
done

echo "FAIL: LDK node did not start within 60s"
ssh -i "$SSH_KEY" "$SERVER" "sudo journalctl -u lsp.service --since '2 minutes ago' --no-pager -n 20"
exit 1
