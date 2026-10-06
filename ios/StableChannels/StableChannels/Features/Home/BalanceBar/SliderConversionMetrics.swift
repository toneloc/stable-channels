import UIKit

/// Calculates label width requirements for slider conversions.
/// Pure and stateless without global mutable caches.
struct SliderConversionMetrics: Equatable {
    let usdPct: Int
    let btcPct: Int
    let sideWidth: CGFloat
    let largestPct: Int

    static func calculate(
        usdPct: Int,
        btcPct: Int,
        font: UIFont = .systemFont(ofSize: 11, weight: .bold)
    ) -> SliderConversionMetrics {
        let largest = max(usdPct, btcPct)
        let digits = max(String(largest).count, 2)
        let sample = digits > 2 ? "000% USD" : "00% USD"
        let attrs: [NSAttributedString.Key: Any] = [.font: font]
        let width = (sample as NSString).size(withAttributes: attrs).width
        let sideWidth = ceil(width) + 4

        return SliderConversionMetrics(
            usdPct: usdPct,
            btcPct: btcPct,
            sideWidth: sideWidth,
            largestPct: largest
        )
    }
}
