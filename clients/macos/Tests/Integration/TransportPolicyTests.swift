import XCTest

@testable import Releash

final class TransportPolicyTests: XCTestCase {
  func testReadsDeadlinesAndBackoffFromBundledDescriptor() throws {
    let policy = try TransportPolicy()
    XCTAssertEqual(policy.timeout, 120)
    XCTAssertEqual(policy.silence, 20)
    XCTAssertTrue((0.8...1.2).contains(policy.backoff(attempt: 0)))
    XCTAssertTrue((96...144).contains(policy.backoff(attempt: 100)))
  }
}
