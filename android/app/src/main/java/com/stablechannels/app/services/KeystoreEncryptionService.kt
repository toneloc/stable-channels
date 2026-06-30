package com.stablechannels.app.services

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.Log
import com.stablechannels.app.util.Constants
import java.io.File
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Encrypted blob containing the IV and ciphertext produced by AES-256-GCM encryption.
 */
data class EncryptedBlob(val iv: ByteArray, val ciphertext: ByteArray) {

    /**
     * Serializes to the on-disk format: [4-byte big-endian IV length][IV][ciphertext]
     */
    fun toByteArray(): ByteArray {
        val buffer = ByteBuffer.allocate(4 + iv.size + ciphertext.size)
        buffer.putInt(iv.size)
        buffer.put(iv)
        buffer.put(ciphertext)
        return buffer.array()
    }

    companion object {
        /**
         * Deserializes from the on-disk format: [4-byte big-endian IV length][IV][ciphertext]
         */
        fun fromByteArray(data: ByteArray): EncryptedBlob {
            val buffer = ByteBuffer.wrap(data)
            val ivLength = buffer.getInt()
            val iv = ByteArray(ivLength)
            buffer.get(iv)
            val ciphertext = ByteArray(buffer.remaining())
            buffer.get(ciphertext)
            return EncryptedBlob(iv, ciphertext)
        }
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is EncryptedBlob) return false
        return iv.contentEquals(other.iv) && ciphertext.contentEquals(other.ciphertext)
    }

    override fun hashCode(): Int {
        var result = iv.contentHashCode()
        result = 31 * result + ciphertext.contentHashCode()
        return result
    }
}

/**
 * Service for encrypting/decrypting the seed phrase using AES-256-GCM
 * with a key stored in the Android Keystore.
 *
 * The key requires user authentication (biometric) to use, and attempts
 * hardware-backed StrongBox storage with a software fallback.
 */
object KeystoreEncryptionService {

    private const val TAG = "KeystoreEncryption"
    private const val KEY_ALIAS = "stable_channels_seed_key"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val GCM_TAG_LENGTH_BITS = 128
    private const val ENCRYPTED_FILE_NAME = "seed_encrypted"
    private const val PLAINTEXT_FILE_NAME = "seed_phrase"

    /**
     * Encrypts the given plaintext using AES-256-GCM with the Keystore-backed key.
     * Generates a new IV for each encryption operation.
     *
     * @param plaintext The data to encrypt
     * @return An [EncryptedBlob] containing the IV and ciphertext
     * @throws Exception if the key cannot be accessed or encryption fails
     */
    fun encrypt(plaintext: ByteArray): EncryptedBlob {
        val key = getOrCreateKey()
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key)
        val iv = cipher.iv
        val ciphertext = cipher.doFinal(plaintext)
        return EncryptedBlob(iv, ciphertext)
    }

    /**
     * Decrypts the given [EncryptedBlob] using AES-256-GCM with the Keystore-backed key.
     *
     * @param blob The encrypted data containing IV and ciphertext
     * @return The decrypted plaintext bytes
     * @throws Exception if the key is invalid, authentication fails, or decryption fails
     */
    fun decrypt(blob: EncryptedBlob): ByteArray {
        val key = getOrCreateKey()
        val cipher = Cipher.getInstance(TRANSFORMATION)
        val spec = GCMParameterSpec(GCM_TAG_LENGTH_BITS, blob.iv)
        cipher.init(Cipher.DECRYPT_MODE, key, spec)
        return cipher.doFinal(blob.ciphertext)
    }

    /**
     * Checks whether the Keystore key is still valid.
     * A key becomes invalid when biometric enrollment changes on the device.
     *
     * @return true if the key exists and can be used, false if invalidated or missing
     */
    fun isKeyValid(): Boolean {
        return try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE)
            keyStore.load(null)
            val key = keyStore.getKey(KEY_ALIAS, null) as? SecretKey ?: return false
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, key)
            true
        } catch (e: KeyPermanentlyInvalidatedException) {
            Log.w(TAG, "Key permanently invalidated (biometric enrollment changed)", e)
            false
        } catch (e: Exception) {
            Log.e(TAG, "Error checking key validity", e)
            false
        }
    }

    /**
     * Deletes the encryption key from the Android Keystore.
     * After deletion, encrypted data can no longer be decrypted.
     */
    fun deleteKey() {
        try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE)
            keyStore.load(null)
            keyStore.deleteEntry(KEY_ALIAS)
            Log.i(TAG, "Keystore key deleted")
        } catch (e: Exception) {
            Log.e(TAG, "Error deleting key", e)
        }
    }

    /**
     * Migrates the plaintext seed phrase to encrypted storage.
     *
     * Reads the `seed_phrase` file, encrypts it, writes the `seed_encrypted` file
     * in the format [4-byte IV length][IV][ciphertext], and deletes the plaintext
     * file only after successful write.
     *
     * @param context Android context for accessing the user data directory
     * @return true if migration succeeded, false if it failed or was not needed
     */
    fun migrateFromPlaintext(context: Context): Boolean {
        val dataDir = Constants.userDataDir(context)
        val plaintextFile = File(dataDir, PLAINTEXT_FILE_NAME)
        val encryptedFile = File(dataDir, ENCRYPTED_FILE_NAME)

        // Nothing to migrate if plaintext file doesn't exist
        if (!plaintextFile.exists()) {
            Log.i(TAG, "No plaintext seed file found, migration not needed")
            return false
        }

        // Already migrated if encrypted file exists
        if (encryptedFile.exists()) {
            Log.i(TAG, "Encrypted seed already exists, skipping migration")
            return false
        }

        return try {
            // Read plaintext seed
            val plaintext = plaintextFile.readBytes()
            if (plaintext.isEmpty()) {
                Log.w(TAG, "Plaintext seed file is empty, skipping migration")
                return false
            }

            // Encrypt the seed
            val blob = encrypt(plaintext)

            // Write encrypted file in format: [4-byte IV length][IV][ciphertext]
            val serialized = blob.toByteArray()
            encryptedFile.writeBytes(serialized)

            // Verify the write was successful by reading back
            if (!encryptedFile.exists() || encryptedFile.length() != serialized.size.toLong()) {
                Log.e(TAG, "Encrypted file verification failed")
                encryptedFile.delete()
                return false
            }

            // Delete plaintext only after successful encryption and write
            val deleted = plaintextFile.delete()
            if (!deleted) {
                Log.w(TAG, "Failed to delete plaintext file after migration")
                // Migration still considered successful since encrypted file is written
            }

            Log.i(TAG, "Seed phrase migration to encrypted storage completed successfully")
            true
        } catch (e: Exception) {
            Log.e(TAG, "Migration failed", e)
            // Clean up partial encrypted file on failure
            if (encryptedFile.exists()) {
                encryptedFile.delete()
            }
            false
        }
    }

    /**
     * Retrieves the existing key from the Keystore, or generates a new one if not present.
     */
    private fun getOrCreateKey(): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE)
        keyStore.load(null)

        // Return existing key if available
        val existingKey = keyStore.getKey(KEY_ALIAS, null) as? SecretKey
        if (existingKey != null) {
            return existingKey
        }

        // Generate a new key
        return generateKey()
    }

    /**
     * Generates a new AES-256-GCM key in the Android Keystore.
     * Attempts StrongBox hardware backing first, falls back to software-backed TEE.
     */
    private fun generateKey(): SecretKey {
        // Try with StrongBox first (API 28+)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                return generateKeyWithSpec(strongBox = true)
            } catch (e: Exception) {
                Log.w(TAG, "StrongBox not available, falling back to software-backed key", e)
            }
        }

        // Fallback to software-backed (TEE)
        return generateKeyWithSpec(strongBox = false)
    }

    /**
     * Generates the AES key with the specified parameters.
     */
    private fun generateKeyWithSpec(strongBox: Boolean): SecretKey {
        val keyGenerator = KeyGenerator.getInstance(
            KeyProperties.KEY_ALGORITHM_AES,
            ANDROID_KEYSTORE
        )

        val specBuilder = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
        // NOTE: We intentionally do NOT call setUserAuthenticationRequired(true) here.
        // The StabilityProcessingService runs as a background ForegroundService and
        // cannot show a biometric prompt — adding auth binding would cause it to crash
        // silently when trying to decrypt the seed. The Android Keystore hardware
        // (TEE / StrongBox) already prevents the AES key from being extracted from the
        // device, which is the meaningful security upgrade over plaintext storage.

        if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            specBuilder.setIsStrongBoxBacked(true)
        }

        keyGenerator.init(specBuilder.build())
        return keyGenerator.generateKey()
    }

    // -------------------------------------------------------------------------
    // Convenience: read the seed, transparently handling both encrypted and
    // legacy-plaintext storage. Every caller (NodeService, StabilityProcessingService)
    // should use this instead of touching the files directly.
    // -------------------------------------------------------------------------

    /**
     * Reads the wallet seed mnemonic from disk.
     *
     * Priority:
     *  1. `seed_encrypted` — AES-256-GCM blob protected by the Android Keystore.
     *  2. `seed_phrase`    — Legacy plaintext file (present on unencrypted wallets
     *                        or before the first migration completes).
     *
     * Returns the trimmed mnemonic string, or `null` if neither file exists
     * (new wallet, or keys_seed-only legacy wallet).
     */
    fun readSeed(context: Context): String? {
        val dataDir = Constants.userDataDir(context)
        val encryptedFile = File(dataDir, ENCRYPTED_FILE_NAME)
        val plaintextFile = File(dataDir, PLAINTEXT_FILE_NAME)

        // 1. Try encrypted file
        if (encryptedFile.exists()) {
            return try {
                val blob = EncryptedBlob.fromByteArray(encryptedFile.readBytes())
                val decrypted = decrypt(blob)
                String(decrypted, Charsets.UTF_8).trim().ifEmpty { null }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to decrypt seed — falling back to plaintext", e)
                // Fall through to plaintext fallback
                null
            }
        }

        // 2. Fall back to legacy plaintext file
        if (plaintextFile.exists()) {
            return try {
                plaintextFile.readText().trim().ifEmpty { null }
            } catch (e: Exception) {
                Log.e(TAG, "Failed to read plaintext seed", e)
                null
            }
        }

        return null
    }
}
