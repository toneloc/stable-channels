import Foundation

/// Decides when Home offers the email prompt: to anyone who has opened the wallet a few times, funded or
/// not. Save or "Not now" ends it; a sheet swiped away comes back after three times as many launches
/// (3, 9, 27 for a new user), at most `maxShows` times. Never while the app is locked: the sheet would draw above the
/// lock.
enum EmailPromptPolicy {
    static let firstLaunch = 3
    static let launchGrowth = 3
    static let maxShows = 3

    static func shouldShow(
        launches: Int,
        timesShown: Int,
        lastShownLaunch: Int,
        hasResponded: Bool,
        isAppLocked: Bool
    ) -> Bool {
        guard !hasResponded, !isAppLocked, timesShown < maxShows else { return false }
        return launches >= max(firstLaunch, lastShownLaunch * launchGrowth)
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
