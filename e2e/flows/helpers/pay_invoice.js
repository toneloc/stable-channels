// Counterparty ("another app" in the demo narrative) pays the invoice the
// flow just copied off the screen with copyTextFrom.
const invoice = maestro.copiedText;
if (!invoice || !invoice.match(/^ln/)) {
    throw new Error(`copiedText is not an invoice: ${invoice}`);
}
const res = http.post(`${HARNESS_API}/pay`, {
    body: JSON.stringify({ invoice: invoice }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /pay failed: ${res.status} ${res.body}`);
}
