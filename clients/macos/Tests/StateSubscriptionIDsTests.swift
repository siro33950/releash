import XCTest

@testable import Releash

final class StateSubscriptionIDsTests: XCTestCase {
  func testLongWorktreePathsUseBoundedOpaqueIDsAndIgnoreReplacedDelivery() {
    var subscriptions = StateSubscriptionIDs()
    let key = String(repeating: "/repository/worktree", count: 20)
    let first = subscriptions.reserve(key)
    XCTAssertLessThanOrEqual(first.id.utf8.count, 128)
    XCTAssertEqual(subscriptions.key(for: first.id), key)
    let second = subscriptions.reserve(key)
    XCTAssertEqual(second.previous, first.id)
    XCTAssertNotEqual(second.id, first.id)
    XCTAssertNil(subscriptions.key(for: first.id))
    XCTAssertEqual(subscriptions.key(for: second.id), key)
    XCTAssertEqual(subscriptions.remove(key), second.id)
    XCTAssertNil(subscriptions.key(for: second.id))
  }
  func testReconnectRemovesOldStreamDeliveries() {
    var subscriptions = StateSubscriptionIDs()
    let one = subscriptions.reserve("workspaces").id
    let two = subscriptions.reserve("workspace-state").id
    subscriptions.reset()
    XCTAssertNil(subscriptions.key(for: one))
    XCTAssertNil(subscriptions.key(for: two))
    XCTAssertNil(subscriptions.reserve("workspaces").previous)
  }
}
