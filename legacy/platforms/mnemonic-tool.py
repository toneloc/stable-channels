import bip39
import secrets  # Make sure to use cryptographically sound randomness

# Generate 128 bits of randomness for a 12-word mnemonic phrase
rand_12 = secrets.randbits(128).to_bytes(16, 'big')  # 16 bytes of randomness
phrase_12 = bip39.encode_bytes(rand_12)

# Generate 256 bits of randomness for a 24-word mnemonic phrase
rand_24 = secrets.randbits(256).to_bytes(32, 'big')  # 32 bytes of randomness
phrase_24 = bip39.encode_bytes(rand_24)

print("Mnemonic Phrase (12 words):", phrase_12)

seed_12 = bip39.phrase_to_seed(phrase_12)[:32]  # Only need 32 bytes
print("Seed (from 12 words):", seed_12.hex())

print("Mnemonic Phrase (24 words):", phrase_24)

seed_24 = bip39.phrase_to_seed(phrase_24)[:32]  # Only need 32 bytes
print("Seed (from 24 words):", seed_24.hex())
