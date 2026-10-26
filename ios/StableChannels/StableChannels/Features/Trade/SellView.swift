import SwiftUI

struct SellView: View {
    let prefillAmountUSD: Double

    init(prefillAmountUSD: Double = 0) {
        self.prefillAmountUSD = prefillAmountUSD
    }

    var body: some View {
        TradeView(action: .sellBTC, prefillAmountUSD: prefillAmountUSD)
    }
}
