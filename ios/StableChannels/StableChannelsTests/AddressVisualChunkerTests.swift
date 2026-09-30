import LDKNode
import XCTest
@testable import StableChannels

final class AddressVisualChunkerTests: XCTestCase {
    func testChunkingP2PKHAddressBoundaryHighlights() {
        let p2pkhAddr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        let chunked = AddressVisualChunker.chunkAddress(p2pkhAddr)

        XCTAssertEqual(chunked.raw, p2pkhAddr)
        XCTAssertEqual(chunked.chunks.count, 9)
        XCTAssertTrue(chunked.chunks[0].isHighlighted)
        XCTAssertTrue(chunked.chunks[1].isHighlighted)
        XCTAssertTrue(chunked.chunks.last?.isHighlighted == true)
    }

    func testFormatDestinationLightningAddressDoesNotChunk() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/.well-known/lnurlp/alice"))
        let dest = SendDestination.lightningAddress(handle: "alice", domain: "example.com", url: url)
        let rep = AddressVisualChunker.formatDestination(dest)

        guard case .lightningAddress(let handle, let domain) = rep else {
            XCTFail("Expected .lightningAddress representation without chunking")
            return
        }
        XCTAssertEqual(handle, "alice")
        XCTAssertEqual(domain, "example.com")
        XCTAssertEqual(rep.rawDestination, "alice@example.com")
    }

    func testFormatDestinationLNURLPayDoesNotChunk() throws {
        let url = try XCTUnwrap(URL(string: "https://ln.tips/service"))
        let dest = SendDestination.lnurlPay(url: url)
        let rep = AddressVisualChunker.formatDestination(dest)

        guard case .lnurl(let host, let rawUrl) = rep else {
            XCTFail("Expected .lnurl representation without chunking")
            return
        }
        XCTAssertEqual(host, "ln.tips")
        XCTAssertEqual(rawUrl, "https://ln.tips/service")
    }

    func testFormatDestinationInvoiceHighlightsBoundaries() {
        let rawInvoice = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        guard let invoice = try? Bolt11Invoice.fromStr(invoiceStr: rawInvoice) else {
            XCTFail("Failed to parse valid bolt11 test invoice")
            return
        }
        let dest = SendDestination.bolt11(invoice: invoice, raw: rawInvoice, amountMsat: 10_000_000)
        let rep = AddressVisualChunker.formatDestination(dest)

        guard case .invoice(let prefix, let middle, let suffix, let raw) = rep else {
            XCTFail("Expected .invoice representation")
            return
        }
        XCTAssertEqual(raw, rawInvoice)
        XCTAssertEqual(prefix, String(rawInvoice.prefix(14)))
        XCTAssertEqual(middle, "········")
        XCTAssertEqual(suffix, String(rawInvoice.suffix(10)))
        XCTAssertTrue(rawInvoice.hasSuffix(suffix))
    }

    func testChunkingSegWitAddressBoundaryHighlights() {
        let segwitAddr = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        let result = AddressVisualChunker.chunkAddress(segwitAddr, chunkSize: 4)

        XCTAssertEqual(result.chunks.count, 11)
        // First 2 chunks (8 characters: "bc1q", "ar0s") highlighted
        XCTAssertEqual(result.chunks[0].text, "bc1q")
        XCTAssertEqual(result.chunks[1].text, "ar0s")
        XCTAssertTrue(result.chunks[0].isHighlighted)
        XCTAssertTrue(result.chunks[1].isHighlighted)

        // Middle chunks should not be highlighted
        for i in 2 ... 8 {
            XCTAssertFalse(result.chunks[i].isHighlighted)
        }

        // Last 2 chunks (6 characters: "wf5m", "dq") highlighted
        XCTAssertEqual(result.chunks[9].text, "wf5m")
        XCTAssertEqual(result.chunks[10].text, "dq")
        XCTAssertTrue(result.chunks[9].isHighlighted)
        XCTAssertTrue(result.chunks[10].isHighlighted)

        // Full reconstruction integrity
        let reconstructed = result.chunks.map(\.text).joined()
        XCTAssertEqual(reconstructed, segwitAddr)
    }

    func testChunkingTinyAddressOneToThreeChars() {
        let single = AddressVisualChunker.chunkAddress("b")
        XCTAssertEqual(single.chunks.count, 1)
        XCTAssertEqual(single.chunks[0].text, "b")
        XCTAssertTrue(single.chunks[0].isHighlighted)

        let triple = AddressVisualChunker.chunkAddress("1Az")
        XCTAssertEqual(triple.chunks.count, 1)
        XCTAssertEqual(triple.chunks[0].text, "1Az")
        XCTAssertTrue(triple.chunks[0].isHighlighted)

        let quad = AddressVisualChunker.chunkAddress("bc1q")
        XCTAssertEqual(quad.chunks.count, 1)
        XCTAssertEqual(quad.chunks[0].text, "bc1q")
        XCTAssertTrue(quad.chunks[0].isHighlighted)
    }

    func testChunkingExactChunkBoundaryMultiples() {
        // 8 chars = 2 chunks (both highlighted)
        let eight = AddressVisualChunker.chunkAddress("12345678")
        XCTAssertEqual(eight.chunks.count, 2)
        XCTAssertTrue(eight.chunks[0].isHighlighted)
        XCTAssertTrue(eight.chunks[1].isHighlighted)

        // 12 chars = 3 chunks (first and last highlighted, middle not)
        let twelve = AddressVisualChunker.chunkAddress("123456789012")
        XCTAssertEqual(twelve.chunks.count, 3)
        XCTAssertTrue(twelve.chunks[0].isHighlighted)
        XCTAssertFalse(twelve.chunks[1].isHighlighted)
        XCTAssertTrue(twelve.chunks[2].isHighlighted)

        // 16 chars = 4 chunks (all 4 highlighted because index < 2 || index >= 2)
        let sixteen = AddressVisualChunker.chunkAddress("1234567890123456")
        XCTAssertEqual(sixteen.chunks.count, 4)
        for chunk in sixteen.chunks {
            XCTAssertTrue(chunk.isHighlighted)
        }

        // 20 chars = 5 chunks (chunks 0, 1, 3, 4 highlighted, chunk 2 unhighlighted)
        let twenty = AddressVisualChunker.chunkAddress("12345678901234567890")
        XCTAssertEqual(twenty.chunks.count, 5)
        XCTAssertTrue(twenty.chunks[0].isHighlighted)
        XCTAssertTrue(twenty.chunks[1].isHighlighted)
        XCTAssertFalse(twenty.chunks[2].isHighlighted)
        XCTAssertTrue(twenty.chunks[3].isHighlighted)
        XCTAssertTrue(twenty.chunks[4].isHighlighted)
    }

    func testAddressWithSurroundingWhitespaceAndNewlines() {
        let dirty = "\n  \t bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq \r\n  "
        let result = AddressVisualChunker.chunkAddress(dirty)
        XCTAssertEqual(result.chunks.count, 11)
        XCTAssertEqual(result.raw, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
        XCTAssertEqual(result.chunks.map(\.text).joined(), "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
    }

    func testInvoiceFormattingThreshold() {
        let rawInvoice = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        guard let invoice = try? Bolt11Invoice.fromStr(invoiceStr: rawInvoice) else {
            XCTFail("Failed to parse invoice")
            return
        }

        let invoice28 = "lnbc123456789012345678901234"
        let shortRep = AddressVisualChunker.formatDestination(.bolt11(
            invoice: invoice,
            raw: invoice28,
            amountMsat: nil
        ))
        if case .invoice(let p, let m, let s, _) = shortRep {
            XCTAssertEqual(p, invoice28)
            XCTAssertEqual(m, "")
            XCTAssertEqual(s, "")
        } else {
            XCTFail("Expected .invoice representation")
        }

        let invoice29 = "lnbc1234567890123456789012345"
        let longRep = AddressVisualChunker.formatDestination(.bolt11(invoice: invoice, raw: invoice29, amountMsat: nil))
        if case .invoice(let p, let m, let s, _) = longRep {
            XCTAssertEqual(p, String(invoice29.prefix(14)))
            XCTAssertEqual(m, "········")
            XCTAssertEqual(s, String(invoice29.suffix(10)))
        } else {
            XCTFail("Expected .invoice representation")
        }
    }
}
