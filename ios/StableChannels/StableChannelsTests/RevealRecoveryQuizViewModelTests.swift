import XCTest
@testable import StableChannels

final class RevealRecoveryQuizViewModelTests: XCTestCase {
    private let testMnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

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
        vm.answerQuestion(questionIndex: 0, optionIndex: 1)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: true, selectedOption: 1))

        // Continue from correct feedback -> moves to Question 2 (index 1)
        vm.continueFromFeedback(questionIndex: 0, isCorrect: true)
        XCTAssertEqual(vm.currentStep, .question(index: 1))
    }

    func testAnswerQuestion1IncorrectTransitionAndRetry() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        // Question 1: incorrect answer is index 0 ("Can get it back for you")
        vm.answerQuestion(questionIndex: 0, optionIndex: 0)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: false, selectedOption: 0))

        // Continue from incorrect feedback -> retries Question 1 (index 0)
        vm.continueFromFeedback(questionIndex: 0, isCorrect: false)
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testFullHappyPathRevealsPhrase() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()

        // Q1: correct (option 1)
        vm.answerQuestion(questionIndex: 0, optionIndex: 1)
        vm.continueFromFeedback(questionIndex: 0, isCorrect: true)
        XCTAssertEqual(vm.currentStep, .question(index: 1))

        // Q2: correct (option 0 - "You're being scammed")
        vm.answerQuestion(questionIndex: 1, optionIndex: 0)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 1, isCorrect: true, selectedOption: 0))

        // Continue from final question -> moves to revealed
        vm.continueFromFeedback(questionIndex: 1, isCorrect: true)
        XCTAssertEqual(vm.currentStep, .revealed)
        XCTAssertFalse(vm.canGoBack)
    }

    func testGoBackFromQuestion1ReturnsToIntro() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        XCTAssertEqual(vm.currentStep, .question(index: 0))

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .intro)
        XCTAssertFalse(vm.canGoBack)
    }

    func testGoBackFromQuestion2ReturnsToQuestion1() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.answerQuestion(questionIndex: 0, optionIndex: 1)
        vm.continueFromFeedback(questionIndex: 0, isCorrect: true)
        XCTAssertEqual(vm.currentStep, .question(index: 1))

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testGoBackFromFeedbackReturnsToQuestion() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.answerQuestion(questionIndex: 0, optionIndex: 0)
        XCTAssertEqual(vm.currentStep, .feedback(questionIndex: 0, isCorrect: false, selectedOption: 0))

        vm.goBack()
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testGoBackWhenRevealedIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.answerQuestion(questionIndex: 0, optionIndex: 1)
        vm.continueFromFeedback(questionIndex: 0, isCorrect: true)
        vm.answerQuestion(questionIndex: 1, optionIndex: 0)
        vm.continueFromFeedback(questionIndex: 1, isCorrect: true)
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

    func testAnswerQuestionOutOfBoundsIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        vm.startQuiz()
        vm.answerQuestion(questionIndex: -1, optionIndex: 0)
        XCTAssertEqual(vm.currentStep, .question(index: 0))

        vm.answerQuestion(questionIndex: 99, optionIndex: 0)
        XCTAssertEqual(vm.currentStep, .question(index: 0))
    }

    func testGoBackFromIntroIsNoOp() {
        let vm = RevealRecoveryQuizViewModel(mnemonic: testMnemonic)
        XCTAssertEqual(vm.currentStep, .intro)
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
