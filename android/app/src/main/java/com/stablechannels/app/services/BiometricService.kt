package com.stablechannels.app.services

import android.content.Context
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

object BiometricService {

    enum class AuthResult {
        SUCCESS,
        CANCELLED,
        NOT_AVAILABLE,
        NOT_ENROLLED,
        BIOMETRY_FAILED,
        DEVICE_CREDENTIAL_FAILED
    }

    enum class BiometricType {
        FINGERPRINT,
        FACE,
        IRIS,
        NONE
    }

    /**
     * Detects the available biometric type on the device.
     * Returns FINGERPRINT, FACE, IRIS, or NONE.
     */
    fun getAvailableBiometricType(context: Context): BiometricType {
        val biometricManager = BiometricManager.from(context)
        val canAuthenticate = biometricManager.canAuthenticate(BIOMETRIC_STRONG)

        if (canAuthenticate != BiometricManager.BIOMETRIC_SUCCESS) {
            return BiometricType.NONE
        }

        // Android doesn't expose a direct API to distinguish fingerprint vs face vs iris
        // through BiometricManager. We check PackageManager for hardware features.
        val pm = context.packageManager
        return when {
            pm.hasSystemFeature("android.hardware.fingerprint") -> BiometricType.FINGERPRINT
            pm.hasSystemFeature("android.hardware.biometrics.face") -> BiometricType.FACE
            pm.hasSystemFeature("android.hardware.biometrics.iris") -> BiometricType.IRIS
            else -> BiometricType.FINGERPRINT // Default to fingerprint if enrolled but feature not declared
        }
    }

    /**
     * Reports whether biometric authentication (BIOMETRIC_STRONG) is available and enrolled.
     */
    fun isBiometricAvailable(context: Context): Boolean {
        val biometricManager = BiometricManager.from(context)
        return biometricManager.canAuthenticate(BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS
    }

    /**
     * Reports whether device credential authentication (PIN/pattern/password) is available.
     */
    fun isDeviceCredentialAvailable(context: Context): Boolean {
        val biometricManager = BiometricManager.from(context)
        return biometricManager.canAuthenticate(DEVICE_CREDENTIAL) == BiometricManager.BIOMETRIC_SUCCESS
    }

    /**
     * Presents the BiometricPrompt and returns the authentication result.
     *
     * Uses suspendCancellableCoroutine to wrap the callback-based BiometricPrompt API.
     * When [allowDeviceCredential] is true, sets BIOMETRIC_STRONG | DEVICE_CREDENTIAL
     * as allowed authenticators, enabling auto-fallback to device credential on lockout.
     */
    suspend fun authenticate(
        activity: FragmentActivity,
        reason: String,
        allowDeviceCredential: Boolean = true
    ): AuthResult {
        // Check availability before showing prompt
        val biometricManager = BiometricManager.from(activity)
        val authenticators = if (allowDeviceCredential) {
            BIOMETRIC_STRONG or DEVICE_CREDENTIAL
        } else {
            BIOMETRIC_STRONG
        }

        val canAuthResult = biometricManager.canAuthenticate(authenticators)
        when (canAuthResult) {
            BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE,
            BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE,
            BiometricManager.BIOMETRIC_ERROR_SECURITY_UPDATE_REQUIRED ->
                return AuthResult.NOT_AVAILABLE

            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED ->
                return AuthResult.NOT_ENROLLED
        }

        return suspendCancellableCoroutine { continuation ->
            val executor = ContextCompat.getMainExecutor(activity)

            val callback = object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    if (continuation.isActive) {
                        continuation.resume(AuthResult.SUCCESS)
                    }
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (continuation.isActive) {
                        val authResult = mapErrorToResult(errorCode, allowDeviceCredential)
                        continuation.resume(authResult)
                    }
                }

                override fun onAuthenticationFailed() {
                    // Called on each failed attempt (e.g., finger not recognized).
                    // BiometricPrompt handles retry internally; we only act on final error/success.
                    // No action needed here — the prompt stays open for retry.
                }
            }

            val biometricPrompt = BiometricPrompt(activity, executor, callback)

            val promptInfoBuilder = BiometricPrompt.PromptInfo.Builder()
                .setTitle("Authentication Required")
                .setSubtitle(reason)
                .setAllowedAuthenticators(authenticators)

            // When DEVICE_CREDENTIAL is included in authenticators, we must NOT set
            // a negative button text (the system provides its own fallback UI).
            if (!allowDeviceCredential) {
                promptInfoBuilder.setNegativeButtonText("Cancel")
            }

            val promptInfo = promptInfoBuilder.build()

            biometricPrompt.authenticate(promptInfo)

            continuation.invokeOnCancellation {
                biometricPrompt.cancelAuthentication()
            }
        }
    }

    /**
     * Maps BiometricPrompt error codes to AuthResult enum values.
     */
    private fun mapErrorToResult(errorCode: Int, allowDeviceCredential: Boolean): AuthResult {
        return when (errorCode) {
            BiometricPrompt.ERROR_USER_CANCELED,
            BiometricPrompt.ERROR_NEGATIVE_BUTTON,
            BiometricPrompt.ERROR_CANCELED ->
                AuthResult.CANCELLED

            BiometricPrompt.ERROR_LOCKOUT,
            BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> {
                // When device credential is allowed, lockout triggers automatic fallback
                // via setAllowedAuthenticators. If we still get lockout here, it means
                // device credential was not allowed or also failed.
                if (allowDeviceCredential) {
                    AuthResult.DEVICE_CREDENTIAL_FAILED
                } else {
                    AuthResult.BIOMETRY_FAILED
                }
            }

            BiometricPrompt.ERROR_NO_BIOMETRICS ->
                AuthResult.NOT_ENROLLED

            BiometricPrompt.ERROR_HW_NOT_PRESENT,
            BiometricPrompt.ERROR_HW_UNAVAILABLE,
            BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL ->
                AuthResult.NOT_AVAILABLE

            else ->
                AuthResult.BIOMETRY_FAILED
        }
    }
}
