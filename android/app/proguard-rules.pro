# LDK Node — bindings are invoked reflectively through JNA; keep everything
-keep class org.lightningdevkit.ldknode.** { *; }

# JNA — loads libldk_node.so; relies on reflection over Structure/Callback types
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.Structure { *; }
-keep class * implements com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Callback { *; }
-dontwarn java.awt.**

# Kotlin Serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt
-keepclassmembers class kotlinx.serialization.json.** { *** Companion; }
-keepclasseswithmembers class kotlinx.serialization.json.** {
    kotlinx.serialization.KSerializer serializer(...);
}
# App classes with @Serializable — keep generated serializers and Companions
-keep,includedescriptorclasses class com.stablechannels.app.**$$serializer { *; }
-keepclassmembers class com.stablechannels.app.** { *** Companion; }
-keepclasseswithmembers class com.stablechannels.app.** {
    kotlinx.serialization.KSerializer serializer(...);
}

# OkHttp / Okio
-dontwarn okhttp3.**
-dontwarn okio.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**

# SLF4J (api-only dependency; no backend at runtime)
-dontwarn org.slf4j.**

# ML Kit barcode scanning
-keep class com.google.mlkit.** { *; }
-dontwarn com.google.mlkit.**

# Firebase Messaging (FCMService is kept via the manifest; keep message types)
-keep class com.google.firebase.messaging.** { *; }

# Biometric (alpha artifact; framework callbacks resolved reflectively)
-keep class androidx.biometric.** { *; }

# CameraX
-keep class androidx.camera.** { *; }
-dontwarn androidx.camera.**
