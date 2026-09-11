// Wait until a harness hold invoice reaches a state: "held" (the app's HTLC
// has arrived and is pending) or "failed" (rejected back to the app).
//
// env: HOLD_HASH (required), HOLD_STATE (default "held"), DEADLINE_SECS (default 60)
const want = typeof HOLD_STATE !== 'undefined' ? HOLD_STATE : 'held';
const deadlineMs = (typeof DEADLINE_SECS !== 'undefined' ? parseInt(DEADLINE_SECS, 10) : 60) * 1000;
const start = Date.now();
let state = null;
while (Date.now() - start < deadlineMs) {
    const res = http.get(`${HARNESS_API}/hold-status?hash=${HOLD_HASH}`);
    if (res.status === 200) {
        state = json(res.body).state;
        if (state === want) break;
    }
    const t = Date.now();
    while (Date.now() - t < 1000) { /* spin */ }
}
if (state !== want) {
    throw new Error(`hold ${HOLD_HASH} is ${state}, expected ${want} within ${deadlineMs / 1000}s`);
}
console.log(`hold ${HOLD_HASH} is ${state}`);
