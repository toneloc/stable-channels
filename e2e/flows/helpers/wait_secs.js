// Busy-wait SECS seconds (GraalJS has no sleep). Used where the flow must let the
// HARNESS-SIDE state advance (e.g. the LSP's ~10s feed poll) while the app is
// deliberately not expected to change.
const secs = typeof SECS !== 'undefined' ? parseInt(SECS, 10) : 5;
const until = Date.now() + secs * 1000;
while (Date.now() < until) { /* spin */ }
