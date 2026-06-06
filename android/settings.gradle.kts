pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
        // ldk-node-android binding, pulled from the toneloc/ldk-node fork's
        // GitHub release (mirrors how iOS pulls the xcframework via SwiftPM).
        // Resolves org.lightningdevkit:ldk-node-android:0.7.5@aar ->
        //   https://github.com/toneloc/ldk-node/releases/download/v0.7.5/ldk-node-android-0.7.5.aar
        ivy {
            url = uri("https://github.com/toneloc/ldk-node/releases/download")
            patternLayout {
                artifact("v[revision]/[artifact]-[revision].[ext]")
            }
            metadataSources { artifact() }
            content { includeModule("org.lightningdevkit", "ldk-node-android") }
        }
    }
}

rootProject.name = "StableChannels"
include(":app")
