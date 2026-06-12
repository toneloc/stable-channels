import UIKit

enum ShareSheetPresenter {
    static func present(items: [Any]) {
        let activityVC = UIActivityViewController(
            activityItems: items,
            applicationActivities: nil
        )

        guard let topVC = topMostViewController() else {
            assertionFailure("ShareSheetPresenter: no view controller in any window scene")
            return
        }

        topVC.present(activityVC, animated: true)
    }

    private static func topMostViewController() -> UIViewController? {
        for scene in UIApplication.shared.connectedScenes {
            guard let windowScene = scene as? UIWindowScene else { continue }
            for window in windowScene.windows where window.isKeyWindow {
                guard var top = window.rootViewController else { continue }
                while let presented = top.presentedViewController {
                    top = presented
                }
                return top
            }
        }
        return nil
    }
}
