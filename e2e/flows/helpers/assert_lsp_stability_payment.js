// Poll the LSP's audit log (via harness /audit-tail) for a stability payment
// sent to the user after the price drop.
const after = SETTLEMENT_AFTER_ISO; // set by set_price_and_mark.js
const expectedPrice = parseFloat(EXPECTED_PRICE_USD);
const expectedAmountMsat = parseInt(EXPECTED_AMOUNT_MSAT, 10);
const amountToleranceMsat = parseInt(AMOUNT_TOLERANCE_MSAT, 10);
const deadlineMs = 60_000;          // E2E LSP tick is 5s; keep restart headroom
const start = Date.now();
let found = false;
let foundAmountMsat = null;
const observedAmountsMsat = [];
let restoredPrice = false;
while (Date.now() - start < deadlineMs && !found) {
    try {
        // A manually restarted harness resets its in-memory price to $100k.
        // Restore the flow's requested price so the LSP and app cannot silently
        // diverge while this assertion waits.
        const infoRes = http.get(`${HARNESS_API}/info`);
        if (infoRes.status === 200) {
            const currentPrice = parseFloat(json(infoRes.body).price);
            if (Math.abs(currentPrice - expectedPrice) > 0.001) {
                const priceRes = http.post(`${HARNESS_API}/price`, {
                    body: JSON.stringify({ price: expectedPrice }),
                    headers: { 'Content-Type': 'application/json' },
                });
                if (priceRes.status === 200 && !restoredPrice) {
                    console.log(`harness price reset detected; restored ${expectedPrice}`);
                    restoredPrice = true;
                }
            }
        }

        const res = http.get(`${HARNESS_API}/audit-tail?n=500`);
        if (res.status === 200) {
            const lines = json(res.body).lines || [];
            for (const line of lines) {
                try {
                    const ev = JSON.parse(line);
                    if (ev.event === 'STABILITY_PAYMENT_SENT'
                        && ev.data && ev.data.direction === 'lsp_to_user'
                        && ev.ts > after) {
                        const amountMsat = Number(ev.data.amount_msat);
                        observedAmountsMsat.push(amountMsat);
                        if (Number.isFinite(amountMsat)
                            && Math.abs(amountMsat - expectedAmountMsat) <= amountToleranceMsat) {
                            found = true;
                            foundAmountMsat = amountMsat;
                            break;
                        }
                    }
                } catch (e) { /* non-JSON line */ }
            }
        }
    } catch (e) { /* tolerate a brief harness restart and retry */ }
    if (!found) {
        // GraalJS has no sleep; busy-wait in coarse steps via polling delay
        const t = Date.now();
        while (Date.now() - t < 5000) { /* spin ~5s between polls */ }
    }
}
if (!found) {
    throw new Error(
        `no ${expectedAmountMsat}±${amountToleranceMsat} msat LSP-to-user stability payment `
        + `observed within ${deadlineMs / 1000}s of ${after}; observed=${observedAmountsMsat.join(',')}`
    );
}
console.log(`LSP-to-user stability payment observed: ${foundAmountMsat} msat`);
