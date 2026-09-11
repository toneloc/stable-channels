// Counterparty produces an onchain address for the app to SEND to.
// Result exposed to the flow as ${output.address}.
const res = http.post(`${HARNESS_API}/address`, {
    body: JSON.stringify({}),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /address failed: ${res.status} ${res.body}`);
}
output.address = json(res.body).address;
