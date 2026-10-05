// Create a harness HOLD invoice (the counterparty keeps the HTLC pending, or
// rejects it on arrival). Sets output.invoice (for the send dance) and
// output.holdHash (for hold_wait.js / hold_claim.js).
//
// env: INVOICE_MSAT (default 15_000_000), HOLD_MODE "hold" (default) | "fail"
const amountMsat = typeof INVOICE_MSAT !== 'undefined' ? parseInt(INVOICE_MSAT, 10) : 15_000_000;
const mode = typeof HOLD_MODE !== 'undefined' ? HOLD_MODE : 'hold';
const res = http.post(`${HARNESS_API}/hold-invoice`, {
    body: JSON.stringify({ amount_msat: amountMsat, mode: mode }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /hold-invoice failed: ${res.status} ${res.body}`);
}
const body = json(res.body);
output.invoice = body.invoice;
output.holdHash = body.payment_hash;
console.log(`hold invoice (${mode}) ${amountMsat} msat, hash ${output.holdHash}`);
