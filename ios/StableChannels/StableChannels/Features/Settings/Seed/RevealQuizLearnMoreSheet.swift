import SwiftUI

// MARK: - Learn More Sheet (Apple Human Interface Guidelines Design)

struct RevealQuizLearnMoreSheet: View {
    @Environment(\.dismiss) private var dismiss
    private let guidelines: [SelfCustodyGuidelineItem]

    init(guidelinesProvider: any SelfCustodyGuidelinesProviderProtocol = DefaultSelfCustodyGuidelinesProvider()) {
        self.guidelines = guidelinesProvider.getGuidelines()
    }

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                VStack(spacing: 0) {
                    ScrollView(showsIndicators: false) {
                        VStack(alignment: .leading, spacing: 32) {
                            // Hero Title Header
                            VStack(alignment: .leading, spacing: 12) {
                                Image(systemName: "shield.checkered")
                                    .font(.system(size: 40))
                                    .foregroundStyle(Color.stablePrimary)

                                Text(String(
                                    localized: "guidelines_hero_title",
                                    defaultValue: "Self-Custody Guidelines"
                                ))
                                .font(.system(size: 28, weight: .bold, design: .default))
                                .foregroundStyle(.white)

                                Text(
                                    String(
                                        localized: "guidelines_hero_desc",
                                        defaultValue: "Your Secret Recovery Phrase gives complete ownership of your Bitcoin and Lightning funds. Follow these core principles to stay secure."
                                    )
                                )
                                .font(.body)
                                .foregroundStyle(.secondary)
                                .lineSpacing(4)
                            }
                            .padding(.top, 16)

                            // Apple HIG Feature List
                            VStack(spacing: 24) {
                                ForEach(guidelines) { item in
                                    appleGuidelineRow(
                                        icon: item.icon,
                                        title: item.title,
                                        desc: item.description
                                    )
                                }
                            }
                        }
                        .padding(.horizontal, 28)
                        .padding(.bottom, 24)
                    }

                    // Bottom Action Button
                    Button {
                        dismiss()
                    } label: {
                        Text(String(localized: "button_done", defaultValue: "Done"))
                            .font(.headline)
                            .foregroundStyle(.black)
                            .frame(maxWidth: .infinity)
                            .frame(height: 52)
                            .background(Color.white)
                            .clipShape(Capsule())
                    }
                    .padding(.horizontal, 24)
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
                            .font(.system(size: 20))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
    }

    private func appleGuidelineRow(
        icon: String,
        title: String,
        desc: String
    ) -> some View {
        HStack(alignment: .top, spacing: 18) {
            Image(systemName: icon)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(Color.stablePrimary)
                .frame(width: 32, alignment: .center)

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                    .foregroundStyle(.primary)

                Text(desc)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineSpacing(3)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
