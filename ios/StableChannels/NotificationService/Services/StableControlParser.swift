import Foundation
import LDKNode

enum StableControlResult {
    case none
    case handled
    case deferToForeground
}

enum StableControlParser {
    static func handleStableControl(
        node: LDKNode.Node,
        db: PaymentDatabase,
        customRecords: [CustomTlvRecord]
    ) -> StableControlResult {
        for record in customRecords where record.typeNum == Constants.stableChannelTLVType {
            if record.value == Data([1]) { continue }
            guard let envelopeStr = String(data: record.value, encoding: .utf8),
                  let envelopeData = envelopeStr.data(using: .utf8),
                  let envelope = try? JSONSerialization.jsonObject(with: envelopeData) as? [String: Any],
                  let payloadStr = envelope["payload"] as? String,
                  let signature = envelope["signature"] as? String,
                  node.verifySignature(
                      msg: Array(payloadStr.utf8),
                      sig: signature,
                      pkey: Constants.lspPubkey
                  ),
                  let payloadData = payloadStr.data(using: .utf8),
                  let payload = try? JSONSerialization.jsonObject(with: payloadData) as? [String: Any],
                  let type = payload["type"] as? String,
                  type == Constants.syncMessageType,
                  let expectedUSD = payload["expected_usd"] as? Double else {
                return .deferToForeground
            }
            let ucid = payload["user_channel_id"] as? String
            return db
                .applySyncMessage(expectedUSD: expectedUSD, payloadUserChannelId: ucid) ? .handled : .deferToForeground
        }
        return .none
    }

    static func isStabilityPayment(_ customRecords: [CustomTlvRecord]) -> Bool {
        customRecords.contains { $0.typeNum == Constants.stableChannelTLVType && $0.value == Data([1]) }
    }
}
