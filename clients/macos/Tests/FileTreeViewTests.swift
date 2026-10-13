import XCTest

@testable import Releash

final class FileTreeViewTests: XCTestCase {
  @MainActor func testDirectoryDeltaReusesItemsAndStopsRemovedSubscriptions() {
    let coordinator = FileTreeView.Coordinator(model: AppModel(), worktree: "/repo")
    var entry = Releash_Client_V1_DirectoryEntry()
    entry.path = "src"; entry.name = "src"; entry.directory = true
    var value = Releash_Client_V1_DirectorySnapshot()
    value.entries = [entry]
    coordinator.apply(value, directory: ".")
    let original = coordinator.items["src"]
    coordinator.subscribe(".")
    coordinator.subscribe("src")
    coordinator.subscribe("src/nested")
    entry.status = "modified"
    value.entries = [entry]; value.delta = true
    coordinator.apply(value, directory: ".")
    XCTAssertTrue(coordinator.items["src"] === original)
    XCTAssertEqual(coordinator.items["src"]?.entry.status, "modified")
    value.entries = []; value.removed = ["src"]
    coordinator.apply(value, directory: ".")
    XCTAssertEqual(Set(coordinator.subscriptions.keys), ["."])
    XCTAssertNil(coordinator.items["src"])
    XCTAssertEqual(coordinator.children["."], [])
    coordinator.stop()
  }

  @MainActor func testDirectoryReplacedWithFileStopsItsSubscription() {
    let coordinator = FileTreeView.Coordinator(model: AppModel(), worktree: "/repo")
    var entry = Releash_Client_V1_DirectoryEntry()
    entry.path = "src"; entry.name = "src"; entry.directory = true
    var value = Releash_Client_V1_DirectorySnapshot()
    value.entries = [entry]
    coordinator.apply(value, directory: ".")
    coordinator.subscribe("src")
    entry.directory = false
    value.entries = [entry]; value.delta = true
    coordinator.apply(value, directory: ".")
    XCTAssertTrue(coordinator.subscriptions.isEmpty)
    XCTAssertFalse(coordinator.items["src"]!.entry.directory)
  }
}
