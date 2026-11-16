import Foundation

/// Decides when Home offers the email prompt: to anyone who has opened the wallet a few times, funded or
/// not. "Not now" or a swipe only dismisses it: it returns after gaps of 3, 9 and 27 launches (launches 3,
/// 12 and 39), then the cycle repeats. Only Save ends it. Never while the app is locked, since the sheet
/// would draw above the lock, and never over another Home sheet.
enum EmailPromptPolicy {
    static let showGaps = [3, 9, 27]

    static func shouldShow(
        launches: Int,
        timesShown: Int,
        lastShownLaunch: Int,
        hasResponded: Bool,
        isAppLocked: Bool,
        isOtherSheetShowing: Bool
    ) -> Bool {
        guard !hasResponded, !isAppLocked, !isOtherSheetShowing else { return false }
        return launches >= lastShownLaunch + showGaps[timesShown % showGaps.count]
    }
}

/// Where the prompt keeps its launch count, when it was shown and the answer; the app stores it in UserDefaults.
@MainActor
protocol EmailPromptStoring: AnyObject {
    var launches: Int { get }
    var hasResponded: Bool { get }
    var timesShown: Int { get }
    var lastShownLaunch: Int { get }
    func recordLaunchOnce()
    func recordShown()
    func markResponded()
}

/// Where the app keeps the last timestamp it signed, so the next signed change is always newer.
@MainActor
protocol EmailTimestampStoring: AnyObject {
    var lastSentTimestamp: Int64 { get set }
}

/// Where the app keeps the address it last saved on the server, so Settings can show it.
@MainActor
protocol SavedEmailStoring: AnyObject {
    var savedEmail: EmailAddress? { get set }
}
