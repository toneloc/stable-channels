import SwiftUI
import CoreImage.CIFilterBuiltins
import QRCode

enum QRCodeUtility {
    // NSCache auto-evicts under memory pressure with configured limits
    private static let cache: NSCache<NSString, UIImage> = {
        let c = NSCache<NSString, UIImage>()
        c.countLimit = 100
        c.totalCostLimit = 50_000_000 // 50MB limit
        return c
    }()

    static func generate(from string: String, size: CGFloat = 512) -> UIImage? {
        let cacheKey = NSString(string: "\(string)_\(Int(size))")
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }

        guard let image = generateQRImage(from: string, size: size) else {
            return nil
        }

        let cost = Int(size * size * 4) // Approximate bytes (RGBA)
        cache.setObject(image, forKey: cacheKey, cost: cost)
        return image
    }

    /// Async variant for SwiftUI .task compatibility
    @MainActor
    static func generateAsync(from string: String, size: CGFloat = 512) async -> UIImage? {
        generate(from: string, size: size)
    }

    static func clearCache() {
        cache.removeAllObjects()
    }

    // BIP-21 URI helper for on-chain addresses
    static func generateBitcoinURI(from address: String, amount: String? = nil) -> String {
        var uri = "bitcoin:\(address)"
        if let amount, !amount.isEmpty {
            uri += "?amount=\(amount)"
        }
        return uri
    }

    private static func generateQRImage(from string: String, size: CGFloat) -> UIImage? {
        do {
            let doc = try QRCode.Document(utf8String: string)

            // Eye style: Peacock corners
            doc.design.shape.eye = QRCode.EyeShape.Peacock()

            // Pixel style
            doc.design.shape.onPixels = QRCode.PixelShape.RoundedPath()
            doc.design.shape.offPixels = QRCode.PixelShape.RoundedPath()

            let cgImage = try doc.cgImage(width: size, height: size)
            return UIImage(cgImage: cgImage)
        } catch {
            // CoreImage fallback
            let context = CIContext()
            let filter = CIFilter.qrCodeGenerator()
            filter.message = Data(string.utf8)
            guard let outputImage = filter.outputImage else { return nil }
            let scaledImage = outputImage.transformed(by: CGAffineTransform(scaleX: 40, y: 40))
            guard let cgImage = context.createCGImage(scaledImage, from: scaledImage.extent) else { return nil }
            return UIImage(cgImage: cgImage)
        }
    }
}
