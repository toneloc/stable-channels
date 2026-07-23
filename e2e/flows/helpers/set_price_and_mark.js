// Set the mocked price AND record the moment, so assert_settlement.js only
// accepts LSP audit events newer than this.
const price = typeof PRICE_USD !== 'undefined' ? parseFloat(PRICE_USD) : 100000.0;
const res = http.post(`${HARNESS_API}/price`, {
    body: JSON.stringify({ price: price }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /price failed: ${res.status} ${res.body}`);
}
output.settlementAfterIso = new Date().toISOString();
console.log(`price set to ${price} at ${output.settlementAfterIso}`);
