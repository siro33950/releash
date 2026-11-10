import XCTest

@testable import Releash

final class FileWorkspaceTests: XCTestCase {
  @MainActor func testOpeningFileUsesRightPaneAndMenuFocusDoesNotChangeCenterTabs() throws {
    // Given
    let model = AppModel()
    let center = try XCTUnwrap(model.panes.controller.allPaneIds.first)
    model.panes.open(.terminal, in: center)
    let before = try model.panes.layout()
    // When
    model.openFile("src/file.swift")
    model.openFile("src/file.swift")
    // Then
    XCTAssertEqual(try model.panes.layout(), before)
    XCTAssertEqual(model.filePanes.controller.allTabIds.count, 1)
    XCTAssertTrue(model.focusedPanes === model.filePanes)
    let id = try XCTUnwrap(model.filePanes.controller.allTabIds.first)
    let view = FileTabPane(model: model, tabID: id, path: "src/file.swift", worktree: "/repo")
    view.setPresentation(mode: "diff", base: "branch-base")
    XCTAssertEqual(view.mode, "diff")
    XCTAssertEqual(view.base, "branch-base")
    model.focusedPanes.closeCurrentTab()
    XCTAssertTrue(model.filePanes.controller.allTabIds.isEmpty)
    XCTAssertEqual(try model.panes.layout(), before)
    model.filesFocused = false
    XCTAssertTrue(model.focusedPanes === model.panes)
  }

  @MainActor func testFilteredSnapshotKeepsServerOrderAndDoesNotSubscribeToDirectories() {
    // Given
    let model = AppModel()
    let coordinator = FileTreeView.Coordinator(model: model, worktree: "/repo")
    var ordinary = Releash_Client_V1_DirectorySnapshot()
    ordinary.entries = [.with { $0.path = "original" }]
    ordinary.order = ["original"]
    coordinator.apply(ordinary, directory: ".")
    var value = Releash_Client_V1_FilteredFilesSnapshot()
    value.directories = [
      .with {
        $0.path = "."
        $0.entries = [.with { $0.path = "second" }, .with { $0.path = "first" }]
      }
    ]
    // When
    coordinator.apply(value)
    coordinator.subscribe(".")
    // Then
    XCTAssertEqual(coordinator.children["."]?.map { $0.entry.path }, ["second", "first"])
    XCTAssertTrue(coordinator.subscriptions.isEmpty)
    coordinator.apply(.init())
    XCTAssertTrue(coordinator.items.isEmpty)
    XCTAssertTrue(coordinator.children.isEmpty)
    coordinator.restoreUnfiltered()
    XCTAssertEqual(coordinator.children["."]?.map { $0.entry.path }, ["original"])
    coordinator.stop()
  }
}
