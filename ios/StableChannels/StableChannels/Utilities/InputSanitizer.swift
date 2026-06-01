enum InputSanitizer {
    /// Keeps digits + at most one dot, trims excess decimals, strips leading zeros.
    /// `"00012.3a."` with `maxDecimals: 2` -> `"12.3"`, `"."` -> `"0."`, `""` -> `""`.
    static func decimal(_ raw: String, maxDecimals: Int = 2) -> String {
        var s = raw
        while s.hasPrefix("0") && s.count > 1 && !s.hasPrefix("0.") {
            s.removeFirst()
        }
        var result = ""
        var seenDot = false
        var decimals = 0
        for ch in s {
            if ch.isNumber {
                if seenDot {
                    decimals += 1
                    if decimals > maxDecimals { continue }
                }
                result.append(ch)
            } else if ch == "." && !seenDot {
                seenDot = true
                result.append(ch)
            }
        }
        if result.isEmpty { return "" }
        if result == "." { return "0." }
        return result
    }
}
