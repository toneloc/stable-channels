// Counterparty creates an invoice for the app to PAY (lightning-send flow).
// Result exposed to the flow as ${output.invoice}.
const amountMsat = typeof INVOICE_MSAT !== 'undefined' ? parseInt(INVOICE_MSAT) : 10_000_000; // 10k sats
const res = http.post(`${HARNESS_API}/invoice`, {
    body: JSON.stringify({ amount_msat: amountMsat }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /invoice failed: ${res.status} ${res.body}`);
}
output.invoice = json(res.body).invoice;
