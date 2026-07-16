// Counterparty sends an onchain payment to the address the flow copied off
// the Fund Wallet screen (regtest addresses are bcrt1...).
const address = maestro.copiedText;
if (!address || !address.match(/^(bc|tb|bcrt)1/)) {
    throw new Error(`copiedText is not an address: ${address}`);
}
const amountSats = typeof SEND_SATS !== 'undefined' ? parseInt(SEND_SATS) : 100_000;
const res = http.post(`${HARNESS_API}/send`, {
    body: JSON.stringify({ address: address, amount_sats: amountSats }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /send failed: ${res.status} ${res.body}`);
}
