import XCTest

@testable import Releash

final class FileWorkspaceTests: XCTestCase {
  private func isolatedDefaults() throws -> (name: String, defaults: UserDefaults) {
    let name = "FileWorkspaceTests.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
    addTeardownBlock { UserDefaults(suiteName: name)?.removePersistentDomain(forName: name) }
    return (name, defaults)
  }

  @MainActor func test_ファイル配置_worktreeごとに保存して別のmodelへ復元する() throws {
    // Given
    let storage = try isolatedDefaults()
    let model = AppModel()
    model.fileLayoutDefaults = storage.defaults
    let first = PaneTab(kind: .file, filePath: "src/first.rs", fileMode: "diff", fileBase: "base")
    let second = PaneTab(kind: .file, filePath: "src/second.rs", fileMode: "file", fileBase: "head")
    let third = PaneTab(kind: .file, filePath: "README.md", fileMode: "diff", fileBase: "head")
    let split = PaneLayout.split(
      id: UUID().uuidString, axis: .horizontal, ratio: 0.4,
      first: .pane(id: UUID().uuidString, tabs: [first, second], active: second.id),
      second: .pane(id: UUID().uuidString, tabs: [third], active: third.id))
    try model.filePanes.restore(split)
    let expectedFirst = try model.filePanes.layout()
    model.selectedWorktree = "/repo/first"
    // When
    model.saveFilePaneLayout()
    try model.filePanes.restore(
      .pane(id: UUID().uuidString, tabs: [third], active: third.id))
    let expectedSecond = try model.filePanes.layout()
    model.selectedWorktree = "/repo/second"
    model.saveFilePaneLayout()
    let restored = AppModel()
    restored.fileLayoutDefaults = try XCTUnwrap(UserDefaults(suiteName: storage.name))
    // Then
    restored.restoreFilePaneLayout("/repo/first")
    XCTAssertEqual(try restored.filePanes.layout(), expectedFirst)
    restored.restoreFilePaneLayout("/repo/second")
    XCTAssertEqual(try restored.filePanes.layout(), expectedSecond)
    restored.restoreFilePaneLayout("/repo/first")
    XCTAssertEqual(try restored.filePanes.layout(), expectedFirst)
  }

  @MainActor func test_ファイル配置_保存が無ければ既存のタブを空の配置に置き換える() throws {
    // Given
    let storage = try isolatedDefaults()
    let model = AppModel()
    model.fileLayoutDefaults = storage.defaults
    model.filePanes.openFile("existing.rs", base: "head")
    XCTAssertFalse(try model.filePanes.layout().isEmpty)
    // When
    model.restoreFilePaneLayout("/repo/missing")
    // Then
    XCTAssertTrue(try model.filePanes.layout().isEmpty)
  }

  @MainActor func test_ファイル配置_保存を読めなければ空の配置に置き換える() throws {
    // Given
    let storage = try isolatedDefaults()
    let model = AppModel()
    model.fileLayoutDefaults = storage.defaults
    model.selectedWorktree = "/repo"
    model.filePanes.openFile("existing.rs", base: "head")
    model.saveFilePaneLayout()
    let saved = try XCTUnwrap(storage.defaults.persistentDomain(forName: storage.name))
    XCTAssertFalse(saved.isEmpty)
    for key in saved.keys {
      storage.defaults.set(Data([0xFF, 0x00, 0xFE]), forKey: key)
    }
    // When
    model.restoreFilePaneLayout("/repo")
    // Then
    XCTAssertTrue(try model.filePanes.layout().isEmpty)
  }

  @MainActor func test_ファイル配置_worktree未選択で保存しても何も変えない() throws {
    // Given
    let storage = try isolatedDefaults()
    storage.defaults.set("unchanged", forKey: "existing")
    let before = try XCTUnwrap(storage.defaults.persistentDomain(forName: storage.name))
    let model = AppModel()
    model.fileLayoutDefaults = storage.defaults
    model.selectedWorktree = nil
    model.filePanes.openFile("unsaved.rs", base: "head")
    let layout = try model.filePanes.layout()
    // When
    model.saveFilePaneLayout()
    // Then
    let after = try XCTUnwrap(storage.defaults.persistentDomain(forName: storage.name))
    XCTAssertEqual(NSDictionary(dictionary: after), NSDictionary(dictionary: before))
    XCTAssertEqual(try model.filePanes.layout(), layout)
  }

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
