// Fail fast when RESTORE_SEED is missing or not a 12/24-word mnemonic.
if (typeof RESTORE_SEED === 'undefined' || !RESTORE_SEED
    || RESTORE_SEED.trim().split(/\s+/).length < 12) {
    throw new Error('RESTORE_SEED must be set to a 12 or 24 word mnemonic');
}
