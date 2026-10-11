// Move the mocked BTC/USD price. This is what makes Step 3 (USD stability)
// deterministically testable: bump the price and the app owes the LSP a
// stability payment; drop it and the LSP owes the app.
const price = typeof PRICE_USD !== 'undefined' ? parseFloat(PRICE_USD) : 100000.0;
const res = http.post(`${HARNESS_API}/price`, {
    body: JSON.stringify({ price: price }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /price failed: ${res.status} ${res.body}`);
}
