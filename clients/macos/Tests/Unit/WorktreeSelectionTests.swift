import XCTest

@testable import Releash

final class WorktreeSelectionTests: XCTestCase {
  func testTasksWithTheSameBranchCanBeSelectedIndependently() {
    var selection = WorktreeSelection()
    selection.set(.task("task-1"), branch: "fix/login", selected: true)
    XCTAssertTrue(selection.contains(.task("task-1")))
    XCTAssertFalse(selection.contains(.task("task-2")))
    selection.set(.task("task-2"), branch: "fix/login", selected: true)
    XCTAssertEqual(selection.branches, ["fix/login", "fix/login"])
    selection.set(.task("task-1"), branch: "fix/login", selected: false)
    XCTAssertFalse(selection.contains(.task("task-1")))
    XCTAssertTrue(selection.contains(.task("task-2")))
    XCTAssertEqual(selection.branches, ["fix/login"])
  }
  func testIssueTaskAndBranchWithSameIdentifierAreIndependent() {
    var selection = WorktreeSelection()
    selection.set(.branch("1204"), branch: "1204", selected: true)
    XCTAssertFalse(selection.contains(.issue(1204)))
    XCTAssertFalse(selection.contains(.task("1204")))
    selection.set(.issue(1204), branch: "feat/issues/1204", selected: true)
    selection.set(.task("1204"), branch: "feat/1204", selected: true)
    XCTAssertEqual(selection.branches, ["1204", "feat/1204", "feat/issues/1204"])
    selection.set(.issue(1204), branch: "feat/issues/1204", selected: false)
    XCTAssertTrue(selection.contains(.branch("1204")))
    XCTAssertTrue(selection.contains(.task("1204")))
    selection.set(.branch("1204"), branch: "1204", selected: false)
    XCTAssertEqual(selection.branches, ["feat/1204"])
    selection.set(.task("1204"), branch: "feat/1204", selected: false)
    XCTAssertTrue(selection.isEmpty)
  }

}
