import UIKit
import Vision

enum QRCodeExtractor {
    /// Cap the longest edge of the input image. Vision can detect a QR in a
    /// few hundred pixels; full-resolution photos waste memory + CPU.
    private static let maxEdge: CGFloat = 1024

    static func extract(from image: UIImage) -> String? {
        guard let cgImage = downscaledCGImage(from: image) else { return nil }
        let request = VNDetectBarcodesRequest()
        request.symbologies = [.qr]
        let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
        try? handler.perform([request])
        return request.results?
            .compactMap(\.payloadStringValue)
            .first(where: { !$0.isEmpty })
    }

    private static func downscaledCGImage(from image: UIImage) -> CGImage? {
        guard let original = image.cgImage else { return nil }
        let w = original.width
        let h = original.height
        let longest = max(w, h)
        guard longest > Int(maxEdge) else { return original }
        let scale = maxEdge / CGFloat(longest)
        let newW = Int(CGFloat(w) * scale)
        let newH = Int(CGFloat(h) * scale)
        let colorSpace = original.colorSpace ?? CGColorSpaceCreateDeviceRGB()
        let bitsPerComponent = original.bitsPerComponent > 0 ? original.bitsPerComponent : 8
        guard let ctx = CGContext(
            data: nil,
            width: newW,
            height: newH,
            bitsPerComponent: bitsPerComponent,
            bytesPerRow: 0,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue
        ) else { return original }
        ctx.interpolationQuality = .high
        ctx.draw(original, in: CGRect(x: 0, y: 0, width: newW, height: newH))
        return ctx.makeImage() ?? original
    }

    /// Normalizes a Bitcoin address according to BIP-173 / BIP-350 specifications.
    /// Native SegWit and Taproot (bc1, tb1, bcrt1) addresses are converted to lowercase.
    /// Base58 addresses (1, 3, 2, m, n) retain their exact case.
    static func normalizeAddress(_ raw: String) -> String {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.contains(where: \.isUppercase) else { return trimmed }

        let lower = trimmed.lowercased()
        if lower.hasPrefix("bc1") || lower.hasPrefix("tb1") || lower.hasPrefix("bcrt1") {
            return lower
        }
        return trimmed
    }

    static func sanitizeAddress(_ raw: String) -> String {
        normalizeAddress(sanitizePaymentURI(raw, scheme: "bitcoin:"))
    }

    static func sanitizeLightningInput(_ raw: String) -> String {
        sanitizePaymentURI(raw, scheme: "lightning:")
    }

    /// Strips both `lightning:` and `bitcoin:` URI schemes (case-insensitive prefix).
    static func sanitizePaymentInput(_ raw: String) -> String {
        normalizeAddress(sanitizeLightningInput(sanitizePaymentURI(raw, scheme: "bitcoin:")))
    }

    private static func sanitizePaymentURI(_ raw: String, scheme: String) -> String {
        guard let line = raw.split(whereSeparator: \.isNewline)
            .first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) else {
            return ""
        }
        var s = String(line)
        if s.contains("\u{00A0}") {
            s = s.replacingOccurrences(of: "\u{00A0}", with: " ")
        }
        s = s.trimmingCharacters(in: .whitespacesAndNewlines)

        if s.range(of: "\(scheme)//", options: [.caseInsensitive, .anchored]) != nil {
            s.removeFirst(scheme.count + 2)
        } else if s.range(of: scheme, options: [.caseInsensitive, .anchored]) != nil {
            s.removeFirst(scheme.count)
        }
        if let q = s.firstIndex(of: "?") {
            s = String(s[..<q])
        }
        return s
    }
}
