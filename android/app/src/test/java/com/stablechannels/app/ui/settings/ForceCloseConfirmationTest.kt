package com.stablechannels.app.ui.settings

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ForceCloseConfirmationTest {
    @Test
    fun confirmsOnlyTheExactPhrase() {
        assertTrue(ForceCloseConfirmation.isConfirmed("FORCE CLOSE"))
        assertTrue(ForceCloseConfirmation.isConfirmed("  FORCE CLOSE "))
        assertFalse(ForceCloseConfirmation.isConfirmed(""))
        assertFalse(ForceCloseConfirmation.isConfirmed("force close"))
        assertFalse(ForceCloseConfirmation.isConfirmed("FORCE"))
        assertFalse(ForceCloseConfirmation.isConfirmed("FORCECLOSE"))
    }
}
