import Foundation

/// Decides when Home offers the email prompt: to anyone who has opened the wallet a few times, funded or
/// not. Only Save ends it; after "Not now" or a swipe the next showing waits the next gap in `showGaps`
/// (launches 3, 12, 39), and the gaps repeat, so a 39-launch stretch never asks more than three times.
/// Never while the app is locked: the sheet would draw above the lock.
enum EmailPromptPolicy {
    static let showGaps = [3, 9, 27]

    static func shouldShow(
        launches: Int,
        timesShown: Int,
        lastShownLaunch: Int,
        hasResponded: Bool,
        isAppLocked: Bool
    ) -> Bool {
        guard !hasResponded, !isAppLocked else { return false }
        return launches >= lastShownLaunch + showGaps[timesShown % showGaps.count]
    }
}

/// Where the prompt keeps its launch count, when it was shown and the answer; the app stores it in UserDefaults.
protocol EmailPromptStoring: AnyObject {
    var launches: Int { get }
    var hasResponded: Bool { get }
    var timesShown: Int { get }
    var lastShownLaunch: Int { get }
    func recordLaunchOnce()
    func recordShown()
    func markResponded()
}

/// Where the app keeps the address it last saved on the server, so Settings can show it.
protocol SavedEmailStoring: AnyObject {
    var savedEmail: EmailAddress? { get set }
}
