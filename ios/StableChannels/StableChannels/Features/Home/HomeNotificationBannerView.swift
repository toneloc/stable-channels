import SwiftUI

struct HomeNotificationBannerView: View {
    var body: some View {
        Button {
            if let url = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(url)
            }
        } label: {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.white)
                VStack(alignment: .leading, spacing: 2) {
                    Text(String(localized: "notifications_disabled", defaultValue: "Notifications Disabled"))
                        .font(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(.white)
                    Text(String(
                        localized: "notifications_disabled_subtitle",
                        defaultValue: "Enable notifications for stability payments"
                    ))
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.9))
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.7))
            }
            .padding(12)
            .background(.red, in: RoundedRectangle(cornerRadius: 12))
        }
    }
}
