package com.stablechannels.app.util

/** Wall-clock time in whole seconds since the Unix epoch. */
fun epochSecondsNow(): Long = System.currentTimeMillis() / 1000
