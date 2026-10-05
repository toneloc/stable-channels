// Assert the LSP recorded a SPLICE_OUT_STABLE_DEDUCTED event after the mark,
// with usd_deducted inside [MIN_DEDUCTED_USD, MAX_DEDUCTED_USD]. Proves the
// protocol-level stable deduction happened, independent of what the client UI
// shows (issue #277 distinguishes a display bug from a books bug).
const after = MARK_ISO;
const minUsd = parseFloat(MIN_DEDUCTED_USD);
const maxUsd = parseFloat(MAX_DEDUCTED_USD);
const deadlineMs = 120_000;
const start = Date.now();
let found = null;
const observed = [];
while (Date.now() - start < deadlineMs && found === null) {
    try {
        const res = http.get(`${HARNESS_API}/audit-tail?n=500`);
        if (res.status === 200) {
            for (const line of (json(res.body).lines || [])) {
                try {
                    const ev = JSON.parse(line);
                    if (ev.event === 'SPLICE_OUT_STABLE_DEDUCTED' && ev.ts > after) {
                        const usd = Number(ev.data && ev.data.usd_deducted);
                        observed.push(usd);
                        if (Number.isFinite(usd) && usd >= minUsd && usd <= maxUsd) {
                            found = usd;
                            break;
                        }
                    }
                } catch (e) { /* non-JSON line */ }
            }
        }
    } catch (e) { /* tolerate a brief harness hiccup and retry */ }
    if (found === null) {
        const t = Date.now();
        while (Date.now() - t < 5000) { /* GraalJS has no sleep; spin ~5s */ }
    }
}
if (found === null) {
    throw new Error(
        `no SPLICE_OUT_STABLE_DEDUCTED in [$${minUsd}, $${maxUsd}] within `
        + `${deadlineMs / 1000}s of ${after}; observed=${observed.join(',')}`);
}
console.log(`LSP deducted $${found} from stable on splice-out`);
