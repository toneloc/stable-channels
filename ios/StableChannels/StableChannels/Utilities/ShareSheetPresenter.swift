import UIKit

enum ShareSheetPresenter {
    static func present(items: [Any]) {
        let activityVC = UIActivityViewController(
            activityItems: items,
            applicationActivities: nil
        )

        guard let windowScene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState == .foregroundActive }),
            let rootVC = windowScene.windows
            .first(where: \.isKeyWindow)?
            .rootViewController else {
            assertionFailure("ShareSheetPresenter: no active foreground window scene with rootViewController")
            return
        }

        var topVC = rootVC
        while let presented = topVC.presentedViewController {
            topVC = presented
        }
        topVC.present(activityVC, animated: true)
    }
}
