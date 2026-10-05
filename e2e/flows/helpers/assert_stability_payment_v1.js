// Assert the presence (or absence) of a signed stability settlement in the
// LSP audit log (harness /audit-tail), matching the current event name
// STABILITY_PAYMENT_V1_SENT. (Superset of assert_lsp_stability_payment.js,
// used by flow 03: this one adds direction and present/absent modes.)
//
// env:
//   SETTLEMENT_AFTER_ISO   only events newer than this count (set_price_and_mark.js)
//   EXPECT                 "present" (default) or "absent"
//   DIRECTION              default "lsp_to_user" (settlements to the app);
//                          "user_to_lsp" for app-paid settlements
//   EXPECTED_AMOUNT_MSAT   present-mode: expected settlement size
//   AMOUNT_TOLERANCE_MSAT  present-mode: accepted deviation
//   EXPECTED_PRICE_USD     present-mode: re-pin the harness price while polling
//                          (a restarted harness resets to $100k)
const after = SETTLEMENT_AFTER_ISO;
const expect = typeof EXPECT !== 'undefined' ? EXPECT : 'present';
const direction = typeof DIRECTION !== 'undefined' ? DIRECTION : 'lsp_to_user';

function matchingEvents() {
    const res = http.get(`${HARNESS_API}/audit-tail?n=500`);
    if (res.status !== 200) return null; // harness hiccup — caller retries
    const out = [];
    for (const line of (json(res.body).lines || [])) {
        try {
            const ev = JSON.parse(line);
            if (ev.event === 'STABILITY_PAYMENT_V1_SENT'
                && ev.data && ev.data.direction === direction
                && ev.ts > after) {
                out.push(Number(ev.data.amount_msat));
            }
        } catch (e) { /* non-JSON line */ }
    }
    return out;
}

if (expect === 'absent') {
    // Single scan: the flow waits out the LSP tick BEFORE calling this, so any
    // wrongly-fired settlement is already in the tail.
    const amounts = matchingEvents();
    if (amounts === null) throw new Error('harness /audit-tail unreachable');
    if (amounts.length > 0) {
        throw new Error(
            `expected NO ${direction} stability settlement after ${after}, `
            + `but observed: ${amounts.join(',')} msat`);
    }
    console.log(`no ${direction} settlement after ${after} — as expected`);
} else {
    const expectedAmountMsat = parseInt(EXPECTED_AMOUNT_MSAT, 10);
    const amountToleranceMsat = parseInt(AMOUNT_TOLERANCE_MSAT, 10);
    const expectedPrice = typeof EXPECTED_PRICE_USD !== 'undefined'
        ? parseFloat(EXPECTED_PRICE_USD) : null;
    const deadlineMs = 60_000; // E2E LSP tick is 5s; keep restart headroom
    const start = Date.now();
    let found = null;
    const observed = [];
    while (Date.now() - start < deadlineMs && found === null) {
        try {
            if (expectedPrice !== null) {
                // Keep the harness price pinned in case it restarted mid-flow.
                const infoRes = http.get(`${HARNESS_API}/info`);
                if (infoRes.status === 200
                    && Math.abs(parseFloat(json(infoRes.body).price) - expectedPrice) > 0.001) {
                    http.post(`${HARNESS_API}/price`, {
                        body: JSON.stringify({ price: expectedPrice }),
                        headers: { 'Content-Type': 'application/json' },
                    });
                }
            }
            const amounts = matchingEvents() || [];
            for (const amountMsat of amounts) {
                observed.push(amountMsat);
                if (Number.isFinite(amountMsat)
                    && Math.abs(amountMsat - expectedAmountMsat) <= amountToleranceMsat) {
                    found = amountMsat;
                    break;
                }
            }
        } catch (e) { /* tolerate a brief harness restart and retry */ }
        if (found === null) {
            const t = Date.now();
            while (Date.now() - t < 5000) { /* GraalJS has no sleep; spin ~5s */ }
        }
    }
    if (found === null) {
        throw new Error(
            `no ${expectedAmountMsat}±${amountToleranceMsat} msat ${direction} stability `
            + `settlement observed within ${deadlineMs / 1000}s of ${after}; `
            + `observed=${observed.join(',')}`);
    }
    console.log(`${direction} stability settlement observed: ${found} msat`);
}
