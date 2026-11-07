import UIKit
import UniformTypeIdentifiers

/// Copies the recovery phrase so it stays on this device and expires on its own.
protocol SecureClipboard {
    func copy(_ text: String)
}

struct SystemSecureClipboard: SecureClipboard {
    var now: () -> Date = Date.init

    /// Local-only keeps the phrase off Universal Clipboard; the system clears it at expiry.
    func options() -> [UIPasteboard.OptionsKey: Any] {
        [
            .expirationDate: now().addingTimeInterval(SeedConstants.clipboardClearSeconds),
            .localOnly: true
        ]
    }

    func copy(_ text: String) {
        UIPasteboard.general.setItems([[UTType.utf8PlainText.identifier: text]], options: options())
    }
}
