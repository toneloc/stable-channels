// Capture the USD balance that copyTextFrom just read from the app.
const raw = maestro.copiedText || "";
const match = raw.replace(/,/g, "").match(/\$?\s*([0-9]+(?:\.[0-9]{1,2})?)/);
if (!match) {
    throw new Error(`copiedText is not a USD balance: ${raw}`);
}

output.offboardBeforeUsd = parseFloat(match[1]);
console.log(`remembered USD balance: ${output.offboardBeforeUsd}`);
