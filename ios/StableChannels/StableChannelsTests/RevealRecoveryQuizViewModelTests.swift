import XCTest
@testable import StableChannels

@MainActor
final class RevealRecoveryQuizViewModelTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

    private func revealedViewModel() -> RevealRecoveryQuizViewModel {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.selectOption(1)
        vm.continueFromFeedback()
        vm.selectOption(0)
        vm.continueFromFeedback()
        return vm
    }

    func testInitialStateIsIntro() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        XCTAssertEqual(vm.currentStep, .intro)
        XCTAssertFalse(vm.canGoBack)
        XCTAssertEqual(vm.questions.count, 2)
        XCTAssertEqual(vm.mnemonic, testMnemonic)
    }

    func testStartQuizMovesToFirstQuestion() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
        XCTAssertTrue(vm.canGoBack)
    }

    func testAnswerQuestion1CorrectTransition() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        // Question 1: correct answer is index 1 ("Can't help you")
        vm.selectOption(1)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: true, selectedOption: 1))

        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 1))
    }

    func testAnswerQuestion1IncorrectTransitionAndRetry() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        // Question 1: incorrect answer is index 0 ("Can get it back for you")
        vm.selectOption(0)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: false, selectedOption: 0))

        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testWrongAnswerOnLastQuestionNeverReveals() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.selectOption(1)
        vm.continueFromFeedback()

        vm.selectOption(1)
        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 1))
    }

    func testFullHappyPathRevealsPhrase() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        vm.selectOption(1)
        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 1))

        // Q2: correct (option 0 - "You're being scammed")
        vm.selectOption(0)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 1, isCorrect: true, selectedOption: 0))

        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .revealed)
        XCTAssertFalse(vm.canGoBack)
    }

    func testSelectOptionOutsideQuestionStepIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.selectOption(1)
        XCTAssertEqual(vm.currentStep, .intro)

        vm.startQuiz()
        vm.selectOption(0)
        vm.selectOption(1)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: false, selectedOption: 0))
    }

    func testContinueOutsideFeedbackStepIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .intro)

        vm.startQuiz()
        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testGoBackFromQuestion1ReturnsToIntro() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .intro)
        XCTAssertEqual(vm.navigationDirection, .backward)
        XCTAssertFalse(vm.canGoBack)
    }

    func testGoBackFromQuestion2ReturnsToQuestion1() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.selectOption(1)
        vm.continueFromFeedback()
        XCTAssertEqual(vm.currentStep, .question(index: 1))

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testGoBackFromFeedbackReturnsToQuestion() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.selectOption(0)

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testForwardActionsResetDirectionAfterGoingBack() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.goBack()
        vm.startQuiz()
        XCTAssertEqual(vm.navigationDirection, .forward)
    }

    func testGoBackWhenRevealedIsNoOp() {
        let vm = revealedViewModel()
        XCTAssertEqual(vm.currentStep, .revealed)
        XCTAssertFalse(vm.canGoBack)

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .revealed)
    }

    func testCustomEmptyQuestionsProviderDirectlyReveals() {
        struct EmptyProvider: RevealQuizQuestionsProviderProtocol {
            func getQuestions() -> [RevealQuizQuestion] { [] }
        }

        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic, questionsProvider: EmptyProvider())
        XCTAssertEqual(vm.questions.count, 0)

        vm.startQuiz()
        XCTAssertEqual(vm.currentStep, .revealed)
        XCTAssertFalse(vm.canGoBack)
    }

    func testGoBackFromIntroIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.goBack()
        XCTAssertEqual(vm.currentStep, .intro)
    }

    func testDefaultGuidelinesProvider() {
        let provider = DefaultSelfCustodyGuidelinesProvider()
        let guidelines = provider.getGuidelines()
        XCTAssertEqual(guidelines.count, 4)
        XCTAssertEqual(guidelines[0].id, "master_key")
        XCTAssertEqual(guidelines[1].id, "no_server_backups")
        XCTAssertEqual(guidelines[2].id, "beware_impersonators")
        XCTAssertEqual(guidelines[3].id, "store_offline")
    }
}
