import Foundation

// MARK: - HTTP Response Parser (Pure Functional Core)

/// Pure functional parser for HTTP/1.1 wire responses with chunked transfer decoding.
enum HTTPResponseParser {
    static let maxResponseBytes: Int = 2 * 1024 * 1024 // 2 MB
    static let maxHeaderBytes: Int = 64 * 1024 // 64 KB

    /// Parses raw network bytes into decoded body data and an HTTPURLResponse metadata object.
    static func parse(data: Data, url: URL, cleanClose: Bool) throws -> (Data, HTTPURLResponse) {
        guard data.count <= maxResponseBytes,
              let crlf2Range = data.range(of: Data([0x0D, 0x0A, 0x0D, 0x0A])),
              crlf2Range.lowerBound <= maxHeaderBytes,
              let headerStr = String(data: data[..<crlf2Range.lowerBound], encoding: .utf8) else {
            throw LNURLError.invalidResponse
        }
        let lines = headerStr.components(separatedBy: "\r\n")
        guard let statusLine = lines.first else { throw LNURLError.invalidResponse }
        let statusParts = statusLine.split(separator: " ")
        guard statusParts.count >= 2,
              statusParts[0].hasPrefix("HTTP/1."),
              let statusCode = Int(statusParts[1]) else {
            throw LNURLError.invalidResponse
        }

        var headers: [String: String] = [:]
        var seenContentLength: String?
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = String(line[..<colon]).trimmingCharacters(in: .whitespaces).lowercased()
            let val = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
            if key == "content-length" {
                if let existing = seenContentLength, existing != val {
                    throw LNURLError.invalidResponse
                }
                seenContentLength = val
            }
            headers[key] = val
        }

        let rawBody = data[crlf2Range.upperBound...]
        let body: Data
        let isChunked = headers["transfer-encoding"]?.lowercased().contains("chunked") == true
        if let lengthStr = headers["content-length"] {
            guard !isChunked, let expectedLen = Int(lengthStr), expectedLen >= 0 else {
                throw LNURLError.invalidResponse
            }
            guard rawBody.count >= expectedLen else { throw LNURLError.invalidResponse }
            body = Data(rawBody.prefix(expectedLen))
        } else if isChunked {
            guard let decoded = decodeChunked(data: Data(rawBody)) else {
                throw LNURLError.invalidResponse
            }
            body = decoded
        } else {
            guard cleanClose else {
                throw LNURLError.invalidResponse
            }
            body = Data(rawBody)
        }

        guard let response = HTTPURLResponse(
            url: url,
            statusCode: statusCode,
            httpVersion: "HTTP/1.1",
            headerFields: headers
        ) else {
            throw LNURLError.invalidResponse
        }
        return (body, response)
    }

    /// Decodes HTTP/1.1 chunked transfer encoded data.
    private static func decodeChunked(data: Data) -> Data? {
        var result = Data()
        var offset = 0
        let count = data.count
        var terminated = false

        while offset < count {
            guard let crlf = data[offset...].range(of: Data([0x0D, 0x0A])) else { return nil }
            guard let hex = String(data: data[offset..<crlf.lowerBound], encoding: .utf8)?
                .trimmingCharacters(in: .whitespaces) else { return nil }
            let cleanHex = hex.split(separator: ";").first.map(String.init) ?? hex
            guard !cleanHex.isEmpty,
                  cleanHex.allSatisfy(\.isHexDigit),
                  let size = Int(cleanHex, radix: 16),
                  size >= 0 else {
                return nil
            }
            if size == 0 {
                terminated = true
                break
            }
            let start = crlf.upperBound
            guard size <= count - start, result.count + size <= maxResponseBytes else { return nil }
            let end = start + size
            result.append(data[start..<end])
            guard end <= count - 2, data[end] == 0x0D, data[end + 1] == 0x0A else { return nil }
            offset = end + 2
        }
        guard terminated else { return nil }
        return result
    }
}
