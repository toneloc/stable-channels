// Settle a held HTLC at the harness counterparty. env: HOLD_HASH (required)
const res = http.post(`${HARNESS_API}/hold-claim`, {
    body: JSON.stringify({ payment_hash: HOLD_HASH }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /hold-claim failed: ${res.status} ${res.body}`);
}
console.log(`hold ${HOLD_HASH} claimed`);
