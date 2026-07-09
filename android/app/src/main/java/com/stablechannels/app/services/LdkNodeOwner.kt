package com.stablechannels.app.services

import java.util.concurrent.atomic.AtomicReference

/**
 * Process-wide guard for LDK node access.
 *
 * The main app and StabilityProcessingService run in the same Android process
 * by default, but each can otherwise build its own Node against the same data
 * directory. LDK channel state is not safe under two live Node instances, so
 * ownership must be held for the full lifetime of any Node and for direct LDK
 * DB mutations such as gossip stripping.
 */
object LdkNodeOwner {
    const val MAIN_APP = "main-app"
    const val STABILITY_SERVICE = "stability-service"

    private val owner = AtomicReference<String?>(null)

    fun tryAcquire(ownerName: String): Boolean =
        owner.compareAndSet(null, ownerName)

    fun release(ownerName: String) {
        owner.compareAndSet(ownerName, null)
    }

    fun currentOwner(): String? = owner.get()

    fun isOwned(): Boolean = owner.get() != null

    fun isOwnedBy(ownerName: String): Boolean = owner.get() == ownerName
}
