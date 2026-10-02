import UIKit

struct SliderConversionMetrics: Equatable {
    let usdPct: Int
    let btcPct: Int
    let sideWidth: CGFloat
    let largestPct: Int

    private static var cachedFont: UIFont?
    private static var cachedTwoDigitWidth: CGFloat = 0
    private static var cachedThreeDigitWidth: CGFloat = 0

    static func calculate(
        usdPct: Int,
        btcPct: Int,
        font: UIFont = .systemFont(ofSize: 11, weight: .bold)
    ) -> SliderConversionMetrics {
        let largest = max(usdPct, btcPct)
        let digits = max(String(largest).count, 2)

        let sideWidth: CGFloat
        if cachedFont == font && cachedTwoDigitWidth > 0 {
            sideWidth = (digits > 2) ? cachedThreeDigitWidth : cachedTwoDigitWidth
        } else {
            cachedFont = font
            let attrs: [NSAttributedString.Key: Any] = [.font: font]
            let sampleTwoUSD = "00% USD"
            let sampleTwoBTC = "00% BTC"
            let w2USD = (sampleTwoUSD as NSString).size(withAttributes: attrs).width
            let w2BTC = (sampleTwoBTC as NSString).size(withAttributes: attrs).width
            cachedTwoDigitWidth = ceil(max(w2USD, w2BTC)) + 4

            let sampleThreeUSD = "000% USD"
            let sampleThreeBTC = "000% BTC"
            let w3USD = (sampleThreeUSD as NSString).size(withAttributes: attrs).width
            let w3BTC = (sampleThreeBTC as NSString).size(withAttributes: attrs).width
            cachedThreeDigitWidth = ceil(max(w3USD, w3BTC)) + 4

            sideWidth = (digits > 2) ? cachedThreeDigitWidth : cachedTwoDigitWidth
        }

        return SliderConversionMetrics(
            usdPct: usdPct,
            btcPct: btcPct,
            sideWidth: sideWidth,
            largestPct: largest
        )
    }
}
