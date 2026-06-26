import SwiftUI
import UIKit

struct NativeTimerAlertModifier: ViewModifier {
    @Binding var isPresented: Bool
    let title: String
    let onConfirm: () async -> Void

    func body(content: Content) -> some View {
        content.background(
            NativeTimerAlertPresenter(
                isPresented: $isPresented,
                title: title,
                onConfirm: onConfirm
            )
        )
    }
}

extension View {
    func nativeTimerAlert(isPresented: Binding<Bool>, title: String,
                          onConfirm: @escaping () async -> Void) -> some View {
        self.modifier(NativeTimerAlertModifier(isPresented: isPresented, title: title, onConfirm: onConfirm))
    }
}

struct NativeTimerAlertPresenter: UIViewControllerRepresentable {
    @Binding var isPresented: Bool
    let title: String
    let onConfirm: () async -> Void

    func makeUIViewController(context _: Context) -> UIViewController {
        let vc = UIViewController()
        vc.view.backgroundColor = .clear
        return vc
    }

    func updateUIViewController(_ uiViewController: UIViewController, context: Context) {
        if isPresented && context.coordinator.alertController == nil {
            context.coordinator.presentAlert(on: uiViewController)
        } else if !isPresented && context.coordinator.alertController != nil {
            context.coordinator.dismissAlert()
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    class Coordinator: NSObject {
        var parent: NativeTimerAlertPresenter
        var alertController: UIAlertController?
        var timer: Timer?
        var secondsRemaining = 10
        var overwriteAction: UIAlertAction?

        init(_ parent: NativeTimerAlertPresenter) {
            self.parent = parent
        }

        func presentAlert(on viewController: UIViewController) {
            secondsRemaining = 10

            let alert = UIAlertController(
                title: parent.title,
                message: baseMessage + "\n\nPlease wait \(secondsRemaining) seconds...",
                preferredStyle: .alert
            )
            self.alertController = alert

            let cancelAction = UIAlertAction(title: "Cancel", style: .cancel) { [weak self] _ in
                self?.cleanup()
            }

            let overwrite = UIAlertAction(title: "Overwrite", style: .destructive) { [weak self] _ in
                guard let self else { return }
                self.stopTimer()
                Task {
                    await self.parent.onConfirm()
                }
                self.parent.isPresented = false
                self.alertController = nil
            }
            overwrite.isEnabled = false
            self.overwriteAction = overwrite

            alert.addAction(cancelAction)
            alert.addAction(overwrite)

            viewController.present(alert, animated: true) { [weak self] in
                self?.startTimer()
            }
        }

        private var baseMessage: String {
            "An existing backup from another device was found in iCloud. Overwriting it will permanently destroy the previous backup. Continue?"
        }

        func startTimer() {
            timer?.invalidate()
            timer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
                self?.updateTimerMessage()
            }
        }

        func updateTimerMessage() {
            guard let alert = alertController else { return }

            secondsRemaining -= 1
            if secondsRemaining > 0 {
                alert.message = baseMessage + "\n\nPlease wait \(secondsRemaining) seconds..."
            } else {
                alert.message = baseMessage
                overwriteAction?.isEnabled = true
                stopTimer()
            }
        }

        func stopTimer() {
            timer?.invalidate()
            timer = nil
        }

        func dismissAlert() {
            alertController?.dismiss(animated: true)
            cleanup()
        }

        func cleanup() {
            stopTimer()
            alertController = nil
            parent.isPresented = false
        }
    }
}
