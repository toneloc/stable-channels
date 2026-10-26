import SwiftUI

struct BuyView: View {
    let prefillAmountUSD: Double

    init(prefillAmountUSD: Double = 0) {
        self.prefillAmountUSD = prefillAmountUSD
    }

    var body: some View {
        TradeView(action: .buyBTC, prefillAmountUSD: prefillAmountUSD)
    }
}
