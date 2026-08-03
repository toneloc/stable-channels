import SwiftUI

struct SeedWordGridView: View {
    let wordFields: [String]
    let isReadOnly: Bool
    let isDisabled: Bool
    let wordCount: Int
    var onWordChanged: ((Int, String) -> Void)?

    private var columns: [GridItem] {
        wordCount == SeedConstants.wordCount12
            ? [GridItem(.flexible()), GridItem(.flexible())]
            : [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())]
    }

    var body: some View {
        LazyVGrid(columns: columns, spacing: 8) {
            ForEach(0..<wordCount, id: \.self) { index in
                wordCell(index: index)
            }
        }
        .animation(.easeInOut, value: wordCount)
    }

    private func wordCell(index: Int) -> some View {
        HStack(spacing: 4) {
            Text("\(index + 1).")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 18, alignment: .trailing)

            if isReadOnly || isDisabled {
                Text(wordFields[index])
                    .font(.system(.caption, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                    .background(Color(uiColor: .tertiarySystemGroupedBackground))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            } else {
                TextField("", text: Binding(
                    get: { wordFields[index] },
                    set: { onWordChanged?(index, $0) }
                ))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.system(.caption, design: .monospaced))
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(Color(uiColor: .tertiarySystemGroupedBackground))
                .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
        .padding(.vertical, 2)
    }
}
