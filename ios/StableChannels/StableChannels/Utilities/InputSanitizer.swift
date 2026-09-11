enum InputSanitizer {
    /// Keeps digits + at most one dot, trims excess decimals, strips leading zeros, prepends zero to leading dot.
    /// `"00012.3a."` with `maxDecimals: 2` -> `"12.3"`, `"."` -> `"0."`, `".5"` -> `"0.5"`, `""` -> `""`.
    static func decimal(_ raw: String, maxDecimals: Int = 2) -> String {
        var slice = raw[...]
        while slice.hasPrefix("0") && slice.count > 1 && !slice.hasPrefix("0.") {
            slice = slice.dropFirst()
        }

        var result = String()
        result.reserveCapacity(slice.count + 2)
        var seenDot = false
        var decimals = 0

        for ch in slice {
            if ch.isNumber {
                if seenDot {
                    decimals += 1
                    if decimals > maxDecimals {
                        continue
                    }
                }
                result.append(ch)
            } else if ch == "." && !seenDot {
                seenDot = true
                if maxDecimals > 0 {
                    result.append(ch)
                }
            }
        }

        if result.isEmpty {
            return ""
        }
        if result.hasPrefix(".") {
            return "0" + result
        }
        return result
    }
}
