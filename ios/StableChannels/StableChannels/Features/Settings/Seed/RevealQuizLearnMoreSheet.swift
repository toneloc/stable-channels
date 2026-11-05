import SwiftUI

// MARK: - Learn More Sheet (Apple Editorial Style)

struct RevealQuizLearnMoreSheet: View {
    @Environment(\.dismiss) private var dismiss
    private let guidelines = SelfCustodyGuidelineItem.all

    var body: some View {
        NavigationStack {
            ZStack {
                Color(uiColor: .systemGroupedBackground).ignoresSafeArea()

                VStack(spacing: 0) {
                    ScrollView(showsIndicators: false) {
                        VStack(alignment: .leading, spacing: 32) {
                            // Apple Website Style Header
                            VStack(alignment: .leading, spacing: 12) {
                                Text(String(
                                    localized: "guidelines_hero_title",
                                    defaultValue: "Self-Custody Guidelines"
                                ))
                                .font(.title.bold())
                                .foregroundStyle(.primary)

                                Text(
                                    String(
                                        localized: "guidelines_hero_desc",
                                        defaultValue: "Your Secret Recovery Phrase gives complete ownership of your Bitcoin and Lightning funds. Follow these core principles to stay secure."
                                    )
                                )
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineSpacing(4)
                            }
                            .padding(.top, 12)

                            // Subtle Divider
                            Divider()

                            // Pure Editorial Sections (No Boxes)
                            VStack(alignment: .leading, spacing: 28) {
                                ForEach(guidelines) { item in
                                    VStack(alignment: .leading, spacing: 8) {
                                        Text(item.title)
                                            .font(.headline)
                                            .foregroundStyle(.primary)

                                        Text(item.description)
                                            .font(.subheadline)
                                            .foregroundStyle(.secondary)
                                            .lineSpacing(4.5)
                                    }
                                }
                            }
                        }
                        .padding(.horizontal, 24)
                        .padding(.bottom, 32)
                    }

                    // Bottom Action Button
                    Button {
                        dismiss()
                    } label: {
                        Text(String(localized: "button_done", defaultValue: "Done"))
                            .font(.headline)
                            .foregroundStyle(Color.revealButtonLabel)
                            .frame(maxWidth: .infinity, minHeight: 50)
                            .background(Color.revealButtonFill)
                            .clipShape(Capsule())
                    }
                    .padding(.horizontal, 20)
                    .padding(.bottom, 20)
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.title3)
                            .foregroundStyle(.secondary)
                    }
                    .accessibilityLabel(String(localized: "button_close", defaultValue: "Close"))
                }
            }
        }
    }
}
