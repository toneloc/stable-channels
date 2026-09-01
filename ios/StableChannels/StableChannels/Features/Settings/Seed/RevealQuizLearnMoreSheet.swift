import SwiftUI

// MARK: - Learn More Sheet (Apple Editorial Style)

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
                            // Apple Website Style Header
                            VStack(alignment: .leading, spacing: 12) {
                                Text(String(
                                    localized: "guidelines_hero_title",
                                    defaultValue: "Self-Custody Guidelines"
                                ))
                                .font(.system(size: 28, weight: .bold))
                                .foregroundStyle(.white)

                                Text(
                                    String(
                                        localized: "guidelines_hero_desc",
                                        defaultValue: "Your Secret Recovery Phrase gives complete ownership of your Bitcoin and Lightning funds. Follow these core principles to stay secure."
                                    )
                                )
                                .font(.system(size: 15))
                                .foregroundStyle(Color(uiColor: .lightGray))
                                .lineSpacing(4)
                            }
                            .padding(.top, 12)

                            // Subtle Divider
                            Rectangle()
                                .fill(Color.white.opacity(0.1))
                                .frame(height: 1)

                            // Pure Editorial Sections (No Boxes)
                            VStack(alignment: .leading, spacing: 28) {
                                ForEach(guidelines) { item in
                                    VStack(alignment: .leading, spacing: 8) {
                                        Text(item.title)
                                            .font(.system(size: 17, weight: .semibold))
                                            .foregroundStyle(.white)

                                        Text(item.description)
                                            .font(.system(size: 15))
                                            .foregroundStyle(Color(white: 0.72))
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
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(.black)
                            .frame(maxWidth: .infinity)
                            .frame(height: 50)
                            .background(Color.white)
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
                            .font(.system(size: 20))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .preferredColorScheme(.dark)
    }
}
