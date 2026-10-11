// Assert presence, absence, or an exact count of LSP audit events, filtered
// SERVER-SIDE by event name and timestamp (harness /audit-tail ?event=&since=).
// Unlike a fixed last-N tail, a SYNC_MESSAGE_FAILED flood can't push the event
// out of view, so "absent" means absent and "present" isn't flaky.
//
// env:
//   EVENTS                 comma list of audit event names (required)
//   AFTER_ISO              only events newer than this ISO time (required)
//   EXPECT                 "present" (default) | "absent" | "count"
//   COUNT                  EXPECT=count: exact number of matching events
//   MATCH                  optional "key=value,key2=value2" filter on the event's data
//   EXPECTED_AMOUNT_MSAT   present: data.amount_msat must be within tolerance
//   AMOUNT_TOLERANCE_MSAT  present: tolerance for the amount (default 0)
//   DEADLINE_SECS          present: how long to poll (default 60)
const events = EVENTS;
const after = AFTER_ISO;
const expect = typeof EXPECT !== 'undefined' ? EXPECT : 'present';
const matchPairs = (typeof MATCH !== 'undefined' && MATCH)
    ? MATCH.split(',').map(kv => kv.split('='))
    : [];
const wantAmount = typeof EXPECTED_AMOUNT_MSAT !== 'undefined'
    ? parseInt(EXPECTED_AMOUNT_MSAT, 10) : null;
const tolerance = typeof AMOUNT_TOLERANCE_MSAT !== 'undefined'
    ? parseInt(AMOUNT_TOLERANCE_MSAT, 10) : 0;
const deadlineMs = (typeof DEADLINE_SECS !== 'undefined' ? parseInt(DEADLINE_SECS, 10) : 60) * 1000;

function matching() {
    const url = `${HARNESS_API}/audit-tail?n=2000`
        + `&event=${encodeURIComponent(events)}&since=${encodeURIComponent(after)}`;
    const res = http.get(url);
    if (res.status !== 200) return null;
    const out = [];
    for (const line of (json(res.body).lines || [])) {
        let ev;
        try { ev = JSON.parse(line); } catch (e) { continue; }
        const d = ev.data || {};
        if (matchPairs.every(([k, v]) => String(d[k]) === v)) out.push(ev);
    }
    return out;
}

function describe(evs) {
    return evs.map(e => `${e.event}(${e.data && e.data.amount_msat !== undefined ? e.data.amount_msat : ''})@${e.ts}`).join(', ');
}

function spin(ms) {
    const t = Date.now();
    while (Date.now() - t < ms) { /* GraalJS has no sleep */ }
}

if (expect === 'absent' || expect === 'count') {
    const evs = matching();
    if (evs === null) throw new Error('harness /audit-tail unreachable');
    const want = expect === 'absent' ? 0 : parseInt(COUNT, 10);
    if (evs.length !== want) {
        throw new Error(`expected ${want} of [${events}] after ${after}, observed ${evs.length}: ${describe(evs)}`);
    }
    console.log(`[${events}] after ${after}: ${evs.length} — as expected`);
} else {
    const start = Date.now();
    let found = null;
    let seen = [];
    while (found === null && Date.now() - start < deadlineMs) {
        const evs = matching() || [];
        seen = evs;
        for (const ev of evs) {
            const amt = Number(ev.data && ev.data.amount_msat);
            if (wantAmount === null || (Number.isFinite(amt) && Math.abs(amt - wantAmount) <= tolerance)) {
                found = ev;
                break;
            }
        }
        if (found === null) spin(3000);
    }
    if (found === null) {
        throw new Error(`no [${events}]${wantAmount !== null ? ` of ${wantAmount}±${tolerance} msat` : ''} `
            + `within ${deadlineMs / 1000}s of ${after}; seen: ${describe(seen)}`);
    }
    output.auditEventIso = found.ts;
    console.log(`observed ${found.event} at ${found.ts}: ${JSON.stringify(found.data)}`);
}
