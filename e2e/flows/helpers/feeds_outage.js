// Take every mock price feed down (DOWN=true) or back up (DOWN=false). Both the
// app and the LSP price from these feeds in E2E. Sets output.outageIso.
const down = typeof DOWN !== 'undefined' ? DOWN === 'true' : true;
const res = http.post(`${HARNESS_API}/feeds/outage`, {
    body: JSON.stringify({ down: down }),
    headers: { 'Content-Type': 'application/json' },
});
if (res.status !== 200) {
    throw new Error(`harness /feeds/outage failed: ${res.status} ${res.body}`);
}
output.outageIso = new Date().toISOString();
console.log(`price feeds ${down ? 'DOWN' : 'up'} at ${output.outageIso}`);
