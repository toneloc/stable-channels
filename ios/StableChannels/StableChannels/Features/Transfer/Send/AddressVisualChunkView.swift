import SwiftUI

/// Renders a destination's human-verifiable representation according to its protocol type.
struct AddressVisualChunkView: View {
    let representation: DestinationVisualRepresentation

    var body: some View {
        switch representation {
        case .onchain(let chunked):
            onchainView(chunked)
        case .invoice(let preview):
            invoiceView(prefix: preview.prefix, middle: preview.middle, suffix: preview.suffix)
        case .lightningAddress(let handle, let domain):
            lightningAddressView(handle: handle, domain: domain)
        case .lnurl(let host, _):
            lnurlView(host: host)
        }
    }

    private func onchainView(_ chunked: ChunkedAddress) -> some View {
        Text(chunkedText(for: chunked))
            .font(.system(.subheadline, design: .monospaced))
            .lineSpacing(4)
    }

    private func invoiceView(prefix: String, middle: String, suffix: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "bolt.fill")
                .font(.subheadline)
                .foregroundStyle(.orange)
            HStack(spacing: 2) {
                Text(prefix)
                    .font(.system(.subheadline, design: .monospaced).weight(.medium))
                    .foregroundStyle(.primary)
                if !middle.isEmpty {
                    Text(middle)
                        .font(.system(.subheadline, design: .monospaced))
                        .foregroundStyle(Color(uiColor: .tertiaryLabel))
                }
                Text(suffix)
                    .font(.system(.subheadline, design: .monospaced).weight(.medium))
                    .foregroundStyle(.primary)
            }
        }
    }

    private func lightningAddressView(handle: String, domain: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "at")
                .font(.subheadline)
                .foregroundStyle(.orange)
            Text(handle)
                .font(.system(.subheadline, design: .rounded).weight(.semibold))
                .foregroundStyle(.primary)
            Text(domain)
                .font(.system(.subheadline, design: .rounded))
                .foregroundStyle(.secondary)
        }
    }

    private func lnurlView(host: String) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "link")
                .font(.subheadline)
                .foregroundStyle(.blue)
            Text(host)
                .font(.system(.subheadline, design: .monospaced).weight(.medium))
                .foregroundStyle(.primary)
        }
    }

    private func chunkedText(for chunked: ChunkedAddress) -> AttributedString {
        var result = AttributedString()
        let total = chunked.chunks.count

        for (idx, chunk) in chunked.chunks.enumerated() {
            var chunkAttr = AttributedString(chunk.text)
            if idx == 0 || idx == total - 1 {
                chunkAttr.font = Font.system(.subheadline, design: .monospaced).weight(.bold)
                chunkAttr.foregroundColor = Color.primary
            } else {
                chunkAttr.font = Font.system(.subheadline, design: .monospaced).weight(.regular)
                chunkAttr.foregroundColor = Color(uiColor: .tertiaryLabel)
            }
            result.append(chunkAttr)
            if idx < total - 1 {
                result.append(AttributedString("  "))
            }
        }
        return result
    }
}
