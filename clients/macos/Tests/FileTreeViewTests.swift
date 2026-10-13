import AppKit
import XCTest

@testable import Releash

final class FileTreeViewTests: XCTestCase {
  @MainActor func testDirectoryDeltaReusesItemsAndStopsRemovedSubscriptions() {
    let coordinator = FileTreeView.Coordinator(model: AppModel(), worktree: "/repo")
    var entry = Releash_Client_V1_DirectoryEntry()
    entry.path = "src"; entry.name = "src"; entry.directory = true
    var value = Releash_Client_V1_DirectorySnapshot()
    value.entries = [entry]; value.order = [entry.path]
    coordinator.apply(value, directory: ".")
    let original = coordinator.items["src"]
    coordinator.subscribe(".")
    coordinator.subscribe("src")
    coordinator.subscribe("src/nested")
    entry.status = "modified"
    value.entries = [entry]; value.order = [entry.path]; value.delta = true
    coordinator.apply(value, directory: ".")
    XCTAssertTrue(coordinator.items["src"] === original)
    XCTAssertEqual(coordinator.items["src"]?.entry.status, "modified")
    value.entries = []; value.order = []; value.removed = ["src"]
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
    value.entries = [entry]; value.order = [entry.path]
    coordinator.apply(value, directory: ".")
    coordinator.subscribe("src")
    entry.directory = false
    value.entries = [entry]; value.order = [entry.path]; value.delta = true
    coordinator.apply(value, directory: ".")
    XCTAssertTrue(coordinator.subscriptions.isEmpty)
    XCTAssertFalse(coordinator.items["src"]!.entry.directory)
  }

  @MainActor func testDirectoryDeltaUsesServerOrderIncludingUnchangedItems() {
    let coordinator = FileTreeView.Coordinator(model: AppModel(), worktree: "/repo")
    var a = Releash_Client_V1_DirectoryEntry(); a.path = "a"; a.name = "a"
    var z = Releash_Client_V1_DirectoryEntry(); z.path = "z"; z.name = "z"
    var value = Releash_Client_V1_DirectorySnapshot()
    value.entries = [a, z]; value.order = ["z", "a"]
    coordinator.apply(value, directory: ".")
    XCTAssertEqual(coordinator.children["."]?.map { $0.entry.path }, ["z", "a"])
    let original = coordinator.items["z"]
    var b = Releash_Client_V1_DirectoryEntry(); b.path = "b"; b.name = "b"
    value.entries = [b]; value.delta = true; value.order = ["a", "b", "z"]
    coordinator.apply(value, directory: ".")
    XCTAssertEqual(coordinator.children["."]?.map { $0.entry.path }, ["a", "b", "z"])
    XCTAssertTrue(coordinator.children["."]?.last === original)
  }

  @MainActor func testExpansionSubscribesOnlyOpenedDirectoryAndCollapseStopsDescendants() {
    let coordinator = FileTreeView.Coordinator(model: AppModel(), worktree: "/repo")
    var src = Releash_Client_V1_DirectoryEntry(); src.path = "src"; src.name = "src"; src.directory = true
    var nested = src; nested.path = "src/nested"; nested.name = "nested"
    var value = Releash_Client_V1_DirectorySnapshot(); value.entries = [src]; value.order = [src.path]
    coordinator.apply(value, directory: ".")
    coordinator.subscribe(".")
    XCTAssertEqual(Set(coordinator.subscriptions.keys), ["."])
    let expand = Notification(name: NSOutlineView.itemDidExpandNotification, userInfo: ["NSObject": coordinator.items["src"]!])
    coordinator.outlineViewItemDidExpand(expand)
    XCTAssertEqual(Set(coordinator.subscriptions.keys), [".", "src"])
    let firstSubscription = coordinator.subscriptions["src"]
    value.entries = [nested]; value.order = [nested.path]
    coordinator.apply(value, directory: "src")
    XCTAssertNil(coordinator.subscriptions["src/nested"])
    coordinator.outlineViewItemDidExpand(Notification(name: NSOutlineView.itemDidExpandNotification, userInfo: ["NSObject": coordinator.items["src/nested"]!]))
    XCTAssertEqual(Set(coordinator.subscriptions.keys), [".", "src", "src/nested"])
    coordinator.outlineViewItemDidCollapse(Notification(name: NSOutlineView.itemDidCollapseNotification, userInfo: ["NSObject": coordinator.items["src"]!]))
    XCTAssertEqual(Set(coordinator.subscriptions.keys), ["."])
    coordinator.outlineViewItemDidExpand(expand)
    XCTAssertEqual(Set(coordinator.subscriptions.keys), [".", "src"])
    XCTAssertNotEqual(coordinator.subscriptions["src"], firstSubscription)
    coordinator.stop()
  }

}
