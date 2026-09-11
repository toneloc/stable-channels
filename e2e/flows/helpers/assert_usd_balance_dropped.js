// Assert the current copied USD balance dropped meaningfully from the remembered value.
const raw = maestro.copiedText || "";
const match = raw.replace(/,/g, "").match(/\$?\s*([0-9]+(?:\.[0-9]{1,2})?)/);
if (!match) {
    throw new Error(`copiedText is not a USD balance: ${raw}`);
}

const before = parseFloat(BEFORE_USD);
const after = parseFloat(match[1]);
const minDrop = typeof MIN_DROP_USD !== "undefined" ? parseFloat(MIN_DROP_USD) : 1;

if (!Number.isFinite(before)) {
    throw new Error(`BEFORE_USD is not a number: ${BEFORE_USD}`);
}
if (!Number.isFinite(after)) {
    throw new Error(`current USD balance is not a number: ${raw}`);
}
if (after > before - minDrop) {
    throw new Error(`USD balance did not drop by at least $${minDrop}: before=${before}, after=${after}`);
}

console.log(`USD balance dropped: before=${before}, after=${after}`);
