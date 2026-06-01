import SwiftUI
import UIKit

enum ShareableQRGenerator {
    // Image dimensions
    private static let imgWidth: CGFloat = 1080
    private static let imgHeight: CGFloat = 1920

    // QR code
    private static let qrSize: CGFloat = 720

    // Header elements
    private static let iconSize: CGFloat = 126
    private static let iconCornerRadius: CGFloat = 18
    private static let textLineHeight: CGFloat = 61
    private static let headerBottomPadding: CGFloat = 62
    private static let headerGap: CGFloat = 0

    // Footer elements
    private static let footerGap: CGFloat = 110
    private static let invoiceToCopyrightGap: CGFloat = 100
    private static let labelToInvoiceGap: CGFloat = 20

    // Lightning invoice truncation
    private static let LN_CHARS_LEFT = 14
    private static let LN_CHARS_RIGHT = 14
    private static let LN_DOTS = "......." // 7 dots

    private static func loadAppIcon() -> UIImage? {
        return UIImage(named: "SplashIcon")
    }

    private static func calculateInvoiceFontSize(for text: String, availableWidth: CGFloat) -> CGFloat {
        let charRatio: CGFloat = 0.6
        let charCount = CGFloat(text.count)
        return floor(availableWidth / (charCount * charRatio))
    }

    private static func calculateOnChainFontSize(for text: String, availableWidth: CGFloat) -> CGFloat {
        var fontSize: CGFloat = 60
        let step: CGFloat = 2
        var font = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
        while text.size(withAttributes: [.font: font]).width > availableWidth && fontSize > 10 {
            fontSize -= step
            font = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
        }
        return fontSize
    }

    static func generateShareImage(qrImage: UIImage, invoice: String, amount: String?,
                                   isOnChain: Bool = false) -> UIImage {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: imgWidth, height: imgHeight))

        return renderer.image { context in
            UIColor.white.setFill()
            context.fill(CGRect(x: 0, y: 0, width: imgWidth, height: imgHeight))

            let headerHeight = iconSize + textLineHeight + headerBottomPadding
            let qrY = imgHeight / 2 - qrSize / 2
            let iconY = qrY - headerHeight - headerGap
            let footerStartY = qrY + qrSize + footerGap

            let textFont = UIFont.systemFont(ofSize: 50, weight: .bold)
            let stableWidth = "Stable".size(withAttributes: [.font: textFont]).width
            let channelsWidth = "Channels".size(withAttributes: [.font: textFont]).width
            let headerTextWidth = max(stableWidth, channelsWidth)
            let textGap: CGFloat = 11
            let totalWidth = iconSize + textGap + headerTextWidth
            let blockCenterX = imgWidth / 2
            let blockStartX = blockCenterX - totalWidth / 2

            // Draw icon
            let icon = loadAppIcon()
            let iconRect = CGRect(
                x: blockStartX,
                y: iconY,
                width: iconSize,
                height: iconSize
            )
            if let loadedIcon = icon {
                let path = UIBezierPath(roundedRect: iconRect, cornerRadius: iconCornerRadius)
                context.cgContext.addPath(path.cgPath)
                context.cgContext.clip()
                loadedIcon.draw(in: iconRect)
                context.cgContext.resetClip()
            } else {
                let path = UIBezierPath(roundedRect: iconRect, cornerRadius: iconCornerRadius)
                UIColor.systemBlue.setFill()
                path.fill()
            }

            // Draw header text
            let textStartX = blockStartX + iconSize + textGap
            let textCenterX = textStartX + headerTextWidth / 2
            let textAttrs: [NSAttributedString.Key: Any] = [.font: textFont, .foregroundColor: UIColor.black]
            "Stable".draw(at: CGPoint(x: textCenterX - stableWidth / 2, y: iconY), withAttributes: textAttrs)
            "Channels".draw(
                at: CGPoint(x: textCenterX - channelsWidth / 2, y: iconY + textLineHeight),
                withAttributes: textAttrs
            )

            // Draw QR
            let qrRect = CGRect(x: (imgWidth - qrSize) / 2, y: qrY, width: qrSize, height: qrSize)
            qrImage.draw(in: qrRect)

            // Draw footer
            var contentY = footerStartY

            if let amountText = amount {
                let amountFont = UIFont.systemFont(ofSize: 58, weight: .bold)
                let amountAttrs: [NSAttributedString.Key: Any] = [.font: amountFont, .foregroundColor: UIColor.black]
                let amountSize = amountText.size(withAttributes: amountAttrs)
                let amountRect = CGRect(
                    x: (imgWidth - amountSize.width) / 2,
                    y: contentY,
                    width: amountSize.width,
                    height: amountSize.height
                )
                amountText.draw(in: amountRect, withAttributes: amountAttrs)
                contentY += amountSize.height + footerGap
            }

            let labelFont = UIFont.systemFont(ofSize: 36, weight: .semibold)
            let labelText = isOnChain ? "Bitcoin Address" : "Lightning Payment"

            if isOnChain {
                let btcSymbol = "₿"
                let btcFont = UIFont.systemFont(ofSize: 36, weight: .semibold)
                let textPortion = "itcoin Address"

                let btcSize = btcSymbol.size(withAttributes: [.font: btcFont])
                let textSize = textPortion.size(withAttributes: [.font: labelFont])
                let totalWidth = btcSize.width + textSize.width

                let btcAttrs: [NSAttributedString.Key: Any] = [
                    .font: btcFont,
                    .foregroundColor: UIColor(red: 1.0, green: 0.6, blue: 0.0, alpha: 1.0)
                ]
                let textAttrs: [NSAttributedString.Key: Any] = [
                    .font: labelFont,
                    .foregroundColor: UIColor.black
                ]

                let startX = (imgWidth - totalWidth) / 2
                btcSymbol.draw(at: CGPoint(x: startX, y: contentY), withAttributes: btcAttrs)
                textPortion.draw(at: CGPoint(x: startX + btcSize.width, y: contentY), withAttributes: textAttrs)
            } else {
                let labelAttrs: [NSAttributedString.Key: Any] = [
                    .font: labelFont,
                    .foregroundColor: UIColor(red: 1.0, green: 0.6, blue: 0.0, alpha: 1.0)
                ]
                let labelSize = labelText.size(withAttributes: labelAttrs)
                let labelRect = CGRect(
                    x: (imgWidth - labelSize.width) / 2,
                    y: contentY,
                    width: labelSize.width,
                    height: 36
                )
                labelText.draw(in: labelRect, withAttributes: labelAttrs)
            }

            contentY += 36 + labelToInvoiceGap

            // Draw invoice
            let availableWidth = qrSize
            let displayInvoice: String
            let invoiceFont: UIFont

            if isOnChain {
                let fontSize = calculateOnChainFontSize(for: invoice, availableWidth: availableWidth)
                invoiceFont = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
                displayInvoice = invoice
            } else {
                let leftPart = String(invoice.prefix(LN_CHARS_LEFT))
                let rightPart = String(invoice.suffix(LN_CHARS_RIGHT))
                displayInvoice = leftPart + LN_DOTS + rightPart

                let fontSize = calculateInvoiceFontSize(for: displayInvoice, availableWidth: availableWidth)
                invoiceFont = UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
            }

            let invoiceAttrs: [NSAttributedString.Key: Any] = [.font: invoiceFont, .foregroundColor: UIColor.black]
            let invoiceSize = displayInvoice.size(withAttributes: invoiceAttrs)
            let invoiceRect = CGRect(
                x: (imgWidth - invoiceSize.width) / 2,
                y: contentY,
                width: invoiceSize.width,
                height: invoiceSize.height
            )
            displayInvoice.draw(in: invoiceRect, withAttributes: invoiceAttrs)

            contentY += invoiceSize.height + invoiceToCopyrightGap

            // Draw copyright
            let copyFont = UIFont.systemFont(ofSize: 22, weight: .medium)
            let copyText = "© StableChannels.com"
            let copyAttrs: [NSAttributedString.Key: Any] = [
                .font: copyFont,
                .foregroundColor: UIColor(white: 0.4, alpha: 1.0)
            ]
            let copySize = copyText.size(withAttributes: copyAttrs)
            let copyRect = CGRect(
                x: (imgWidth - copySize.width) / 2,
                y: contentY,
                width: copySize.width,
                height: copySize.height
            )
            copyText.draw(in: copyRect, withAttributes: copyAttrs)
        }
    }
}
