// Record the current time so later audit-log assertions only accept newer events.
output.markIso = new Date().toISOString();
console.log(`marked ${output.markIso}`);
