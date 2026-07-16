// Poll the LSP's audit log (via harness /audit-tail) for an incoming payment
// event newer than SETTLEMENT_AFTER_ISO — the stability settlement the app
// owes after a price rise. Untagged foreground sends audit PAYMENT_RECEIVED;
// TLV-tagged ones audit MESSAGE_RECEIVED (post issue-#161 fix).
const after = SETTLEMENT_AFTER_ISO; // set by set_price_and_mark.js
const deadlineMs = 200_000;         // stability tick 60s + cooldown headroom
const start = Date.now();
let found = null;
while (Date.now() - start < deadlineMs && !found) {
    const res = http.get(`${HARNESS_API}/audit-tail?n=80`);
    if (res.status === 200) {
        const lines = json(res.body).lines || [];
        for (const line of lines) {
            try {
                const ev = JSON.parse(line);
                if ((ev.event === 'PAYMENT_RECEIVED' || ev.event === 'MESSAGE_RECEIVED')
                    && ev.ts > after) {
                    found = ev.event;
                    break;
                }
            } catch (e) { /* non-JSON line */ }
        }
    }
    if (!found) {
        // GraalJS has no sleep; busy-wait in coarse steps via polling delay
        const t = Date.now();
        while (Date.now() - t < 5000) { /* spin ~5s between polls */ }
    }
}
if (!found) {
    throw new Error(`no settlement observed on the LSP within ${deadlineMs / 1000}s of ${after}`);
}
console.log(`settlement observed: ${found}`);
