import XCTest
@testable import StableChannels

final class Base58CheckTests: XCTestCase {
    func testValidP2PKHMainnet() {
        let address = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        XCTAssertTrue(Base58Check.verify(address))
    }

    func testValidP2SHMainnet() {
        let address = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
        XCTAssertTrue(Base58Check.verify(address))
    }

    func testValidTestnetP2PKH() {
        let address = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn"
        XCTAssertTrue(Base58Check.verify(address))
    }

    func testValidTestnetP2SH() {
        let address = "2MsFDzHRUAMpjHxKyoEHU3aMCMsVtMqs1PV"
        XCTAssertTrue(Base58Check.verify(address))
    }

    func testInvalidChecksum() {
        // Change last character 'a' to 'b'
        let corrupted = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"
        XCTAssertFalse(Base58Check.verify(corrupted))
    }

    func testInvalidCharactersNotInAlphabet() {
        // '0', 'O', 'I', 'l' are omitted in Base58
        let invalid0 = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfN0"
        let invalidO = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNO"
        let invalidI = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNI"
        let invalidL = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNl"

        XCTAssertFalse(Base58Check.verify(invalid0))
        XCTAssertFalse(Base58Check.verify(invalidO))
        XCTAssertFalse(Base58Check.verify(invalidI))
        XCTAssertFalse(Base58Check.verify(invalidL))
    }

    func testEmptyAndWhitespaceInput() {
        XCTAssertFalse(Base58Check.verify(""))
        XCTAssertFalse(Base58Check.verify("   \n\t "))
    }

    func testTruncatedAddress() {
        let short = "1A1zP1eP5QGefi2"
        XCTAssertFalse(Base58Check.verify(short))
    }
}
