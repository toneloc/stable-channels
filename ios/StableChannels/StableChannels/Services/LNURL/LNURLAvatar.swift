import Foundation
import ImageIO

/// Decodes an LNURL payee avatar defensively. The bytes come from whoever runs the payee's
/// server, so the decode is bounded before any pixels are allocated.
enum LNURLAvatar {
    /// Largest encoded avatar accepted (bytes).
    static let maxEncodedBytes = 256 * 1024
    /// Longest source side accepted, checked from the header before decoding.
    static let maxSourceSide = 4096
    /// Longest side of the decoded thumbnail; covers a 36pt avatar at 3x.
    static let maxPixelSize = 128

    private static let pngSignature: [UInt8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    private static let jpegSignature: [UInt8] = [0xFF, 0xD8, 0xFF]

    static func thumbnail(from data: Data) -> CGImage? {
        guard data.count <= maxEncodedBytes,
              data.starts(with: pngSignature) || data.starts(with: jpegSignature),
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              (1...maxSourceSide).contains(width), (1...maxSourceSide).contains(height) else {
            return nil
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
            kCGImageSourceShouldCacheImmediately: true
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}
