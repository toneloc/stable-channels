// Mine regtest blocks so onchain deposits / splices / closes confirm.
const blocks = typeof MINE_BLOCKS !== 'undefined' ? parseInt(MINE_BLOCKS) : 6;
const res = http.post(`${HARNESS_API}/mine`, {
    body: JSON.stringify({ blocks: blocks }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /mine failed: ${res.status} ${res.body}`);
}
