import Foundation

/// Active step in the sequential Send workflow.
enum SendFlowStep: Equatable, Sendable {
    case recipient
    case amount
    case confirm
    case success
}
