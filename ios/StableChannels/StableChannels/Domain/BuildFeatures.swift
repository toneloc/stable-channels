import Foundation

/// Whether this build includes stability. Set at compile time only, so nothing remote can turn it on.
enum BuildFeatures {
    #if STABILITY
        static let stability = true
    #else
        static let stability = false
    #endif
}
