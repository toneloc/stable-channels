import Foundation

/// Remembers how often the app launched, when the email prompt was shown, whether the user answered it,
/// and the address last saved on the server.
final class EmailPromptStore: EmailPromptStoring, SavedEmailStoring, @unchecked Sendable {
    static let shared = EmailPromptStore()

    private static let launchesKey = "emailPromptLaunchCount"
    private static let respondedKey = "emailPromptResponded"
    private static let timesShownKey = "emailPromptTimesShown"
    private static let lastShownLaunchKey = "emailPromptLastShownLaunch"
    private static let savedEmailKey = "savedEmailAddress"

    private let defaults: UserDefaults
    private var recordedThisProcess = false

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    var launches: Int {
        defaults.integer(forKey: Self.launchesKey)
    }

    var hasResponded: Bool {
        defaults.bool(forKey: Self.respondedKey)
    }

    var timesShown: Int {
        defaults.integer(forKey: Self.timesShownKey)
    }

    var lastShownLaunch: Int {
        defaults.integer(forKey: Self.lastShownLaunchKey)
    }

    var savedEmail: EmailAddress? {
        get { defaults.string(forKey: Self.savedEmailKey).flatMap(EmailAddress.init) }
        set {
            if let newValue {
                defaults.set(newValue.value, forKey: Self.savedEmailKey)
            } else {
                defaults.removeObject(forKey: Self.savedEmailKey)
            }
        }
    }

    /// Counts this process once, however many times Home appears.
    func recordLaunchOnce() {
        guard !recordedThisProcess else { return }
        recordedThisProcess = true
        defaults.set(launches + 1, forKey: Self.launchesKey)
    }

    func recordShown() {
        defaults.set(timesShown + 1, forKey: Self.timesShownKey)
        defaults.set(launches, forKey: Self.lastShownLaunchKey)
    }

    func markResponded() {
        defaults.set(true, forKey: Self.respondedKey)
    }
}
