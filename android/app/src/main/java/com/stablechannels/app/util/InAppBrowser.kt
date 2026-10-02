package com.stablechannels.app.util

import android.content.Context
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent

/** Opens a web link in an in-app browser sheet (Custom Tab). */
fun Context.openInAppBrowser(url: String) {
    CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(this, Uri.parse(url))
}
