import Foundation

enum EsploraError: LocalizedError {
    case invalidResponse
    case httpError(Int)
    var errorDescription: String? {
        switch self {
        case .invalidResponse: return "Invalid response from Esplora"
        case .httpError(let code): return "Esplora HTTP error: \(code)"
        }
    }
}
