import Bonsplit
import XCTest

@testable import Releash

final class PaneWorkspaceTests: XCTestCase {
  @MainActor func testRestorationPreservesFocusAndTabIdentifiersOnNonUUIDSecondPane() throws {
    let workspace = PaneWorkspace()
    let layout = PaneLayout.split(
      id: "split", axis: .horizontal, ratio: 0.5,
      first: .pane(
        id: "first", tabs: [PaneTab(id: "file", kind: .file)], active: "file"),
      second: .pane(
        id: "pane",
        tabs: [
          PaneTab(id: "terminal", kind: .terminal),
          PaneTab(id: "session-example", kind: .session, agentSessionID: "example"),
        ], active: "terminal"))
    try workspace.restore(layout)
    let second = workspace.controller.allPaneIds[1]
    workspace.controller.focusPane(second)
    let tabs = workspace.controller.allTabIds
    let selected = try XCTUnwrap(workspace.controller.selectedTab(inPane: second)?.id)
    XCTAssertEqual(workspace.controller.focusedPaneId, second)

    try workspace.restore(layout)

    XCTAssertEqual(workspace.controller.focusedPaneId, second)
    XCTAssertEqual(workspace.controller.allTabIds, tabs)
    XCTAssertEqual(workspace.controller.selectedTab(inPane: second)?.id, selected)
    XCTAssertEqual(try workspace.layout(), layout)
  }

  @MainActor func testServerAddedTabAppendsAtEndAndKeepsFocusAndSelection() throws {
    let workspace = PaneWorkspace()
    let first = PaneTab(id: "first", kind: .session, agentSessionID: "first")
    let second = PaneTab(id: "second", kind: .session, agentSessionID: "second")
    try workspace.restore(
      .split(
        id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: "left", tabs: [first, second], active: first.id),
        second: .pane(id: "right", tabs: [], active: nil)))
    let left = workspace.controller.allPaneIds[0]
    let right = workspace.controller.allPaneIds[1]
    workspace.controller.focusPane(right)
    let leftTabs = workspace.controller.tabs(inPane: left).map(\.id)
    let selected = workspace.controller.selectedTab(inPane: left)?.id

    let added = PaneTab(id: "session-added", kind: .session, agentSessionID: "added")
    try workspace.restore(
      .split(
        id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: "left", tabs: [first, second, added], active: first.id),
        second: .pane(id: "right", tabs: [], active: nil)))

    XCTAssertEqual(workspace.controller.allPaneIds, [left, right])
    XCTAssertEqual(Array(workspace.controller.tabs(inPane: left).map(\.id).prefix(2)), leftTabs)
    XCTAssertEqual(workspace.controller.tabs(inPane: left).count, 3)
    XCTAssertEqual(workspace.controller.selectedTab(inPane: left)?.id, selected)
    XCTAssertEqual(workspace.controller.focusedPaneId, right)
    XCTAssertEqual(try workspace.layout().tabs.map(\.id), ["first", "second", "session-added"])
  }

  @MainActor func testReorderedTabsAreRestoredInTheReceivedOrder() throws {
    let workspace = PaneWorkspace()
    let first = PaneTab(id: "first", kind: .session, agentSessionID: "first")
    let second = PaneTab(id: "second", kind: .session, agentSessionID: "second")
    try workspace.restore(.pane(id: "pane", tabs: [first, second], active: first.id))

    try workspace.restore(.pane(id: "pane", tabs: [second, first], active: first.id))

    XCTAssertEqual(try workspace.layout().tabs.map(\.id), ["second", "first"])
  }

  @MainActor func testBoundSessionTabDropsItsPendingMapping() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.pane(id: "pane", tabs: [], active: nil))
    workspace.openPendingSession(id: "node", title: "Pending")
    let pending = try XCTUnwrap(workspace.controller.allTabIds.first)
    let bound = PaneTab(id: pending.id.uuidString, kind: .session, agentSessionID: "agent")

    try workspace.restore(.pane(id: "pane", tabs: [bound], active: bound.id))
    workspace.openPendingSession(id: "node", title: "Pending again")

    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    XCTAssertEqual(workspace.tab(pending)?.agentSessionID, "agent")
  }

  @MainActor func testRestorationPreservesFocusOnSecondPane() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    let first = try XCTUnwrap(workspace.controller.allPaneIds.first)
    let second = try XCTUnwrap(workspace.controller.splitPane(first, orientation: .vertical))
    workspace.controller.focusPane(second)
    let layout = try workspace.layout()
    XCTAssertEqual(workspace.controller.focusedPaneId, second)

    try workspace.restore(layout)

    XCTAssertEqual(workspace.controller.focusedPaneId, second)
    XCTAssertEqual(try workspace.layout(), layout)
  }

  @MainActor func testRestorationFocusesFirstPaneWhenFocusedPaneIsRemoved() throws {
    let workspace = PaneWorkspace()
    let layout = PaneLayout.pane(id: UUID().uuidString, tabs: [], active: nil)
    try workspace.restore(layout)
    let first = try XCTUnwrap(workspace.controller.allPaneIds.first)
    let second = try XCTUnwrap(workspace.controller.splitPane(first, orientation: .vertical))
    workspace.controller.focusPane(second)
    XCTAssertEqual(workspace.controller.focusedPaneId, second)

    try workspace.restore(layout)

    XCTAssertEqual(workspace.controller.allPaneIds, [first])
    XCTAssertEqual(workspace.controller.focusedPaneId, first)
  }

  @MainActor func testRestoresNestedLayoutWithAllKindsAndOriginalIdentifiers() throws {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(id: $0.rawValue, kind: $0) }
    let layout = PaneLayout.split(
      id: "outer", axis: .horizontal, ratio: 0.3,
      first: .pane(id: "a", tabs: [tabs[0]], active: tabs[0].id),
      second: .split(
        id: "inner", axis: .vertical, ratio: 0.7,
        first: .pane(id: "b", tabs: [tabs[1]], active: tabs[1].id),
        second: .pane(id: "c", tabs: Array(tabs[2...]), active: tabs[2].id)))
    let workspace = PaneWorkspace()
    try workspace.restore(layout)
    XCTAssertEqual(try workspace.layout(), layout)
    let restored = PaneWorkspace()
    try restored.restore(workspace.layout())
    XCTAssertEqual(try restored.layout(), layout)
    XCTAssertEqual(restored.controller.allTabIds.count, PaneTab.Kind.allCases.count)
    XCTAssertEqual(restored.controller.allPaneIds.count, 3)
  }

  @MainActor func testOpeningAndClosingAllKindsPreservesOtherPaneAndCanReopenLastPane() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    let first = try XCTUnwrap(workspace.controller.allPaneIds.first)
    for kind in PaneTab.Kind.allCases { workspace.open(kind, in: first) }
    let second = try XCTUnwrap(workspace.controller.splitPane(first, orientation: .vertical))
    workspace.open(.file, in: second)
    let untouched = workspace.controller.tabs(inPane: second)
    workspace.controller.focusPane(first)
    for _ in PaneTab.Kind.allCases { workspace.closeCurrentTab() }
    XCTAssertEqual(workspace.controller.allPaneIds, [second])
    XCTAssertEqual(workspace.controller.tabs(inPane: second), untouched)
    workspace.closeCurrentTab()
    XCTAssertEqual(workspace.controller.allPaneIds.count, 1)
    workspace.open(.command, in: second)
    XCTAssertEqual(try workspace.layout().tabs.map(\.kind), [.command])
  }

  @MainActor func testRestorationPreservesUnselectedTabsAndUUIDLetterCase() throws {
    let id = "AAAAAAAA-0000-4000-8000-000000000001"
    let tabs = [PaneTab(id: id, kind: .terminal), PaneTab(id: id.lowercased(), kind: .file)]
    let layout = PaneLayout.pane(id: "pane", tabs: tabs, active: nil)
    let workspace = PaneWorkspace()
    try workspace.restore(layout)
    XCTAssertEqual(try workspace.layout(), layout)
  }

  @MainActor func testInvalidRestorationDoesNotDestroyCurrentTabs() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.open(.terminal, in: try XCTUnwrap(workspace.controller.allPaneIds.first))
    let before = try workspace.layout()
    let malformed = PaneLayout.split(
      id: "bad", axis: .horizontal, ratio: .nan,
      first: .empty(), second: .empty())
    XCTAssertThrowsError(try workspace.restore(malformed))
    XCTAssertEqual(try workspace.layout(), before)
  }
  @MainActor func test_フォーカス_選択済み領域を押しても通知する() throws {
    // Given
    let workspace = PaneWorkspace()
    let pane = try XCTUnwrap(workspace.controller.focusedPaneId)
    var notifications = 0
    workspace.onFocus = { notifications += 1 }
    // When
    workspace.controller.focusPane(pane)
    workspace.controller.focusPane(pane)
    // Then
    XCTAssertEqual(notifications, 2)
  }

  @MainActor func test_配置復元_ファイル領域の操作対象を変えない() throws {
    for split in [false, true] {
      // Given
      let model = AppModel()
      let terminal = PaneTab(id: "terminal", kind: .terminal)
      let center = PaneLayout.pane(id: "center", tabs: [terminal], active: terminal.id)
      try model.panes.restore(center)
      model.panes.onFocus = { [weak model] in model?.filesFocused = false }
      model.filePanes.onFocus = { [weak model] in model?.filesFocused = true }
      model.openFile("first.rs")
      let updated =
        split
        ? PaneLayout.split(
          id: "split", axis: .horizontal, ratio: 0.5, first: center,
          second: .pane(id: "other", tabs: [], active: nil))
        : PaneLayout.pane(
          id: "center", tabs: [terminal, PaneTab(id: "added", kind: .terminal)],
          active: terminal.id)
      // When
      try model.panes.restore(updated)
      // Then
      XCTAssertTrue(model.focusedPanes === model.filePanes)
      model.focusedPanes.closeCurrentTab()
      XCTAssertTrue(model.filePanes.controller.allTabIds.isEmpty)
      XCTAssertEqual(try model.panes.layout(), updated)
      model.panes.controller.focusPane(try XCTUnwrap(model.panes.controller.focusedPaneId))
      XCTAssertTrue(model.focusedPanes === model.panes)
      model.filePanes.controller.focusPane(try XCTUnwrap(model.filePanes.controller.focusedPaneId))
      XCTAssertTrue(model.focusedPanes === model.filePanes)
    }
  }

  @MainActor func test_ファイルタブ_フォーカス領域内だけで再利用し表示を保持する() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.openFile("src/file.swift", base: "head")
    let first = try XCTUnwrap(workspace.controller.allTabIds.first)
    workspace.setFilePresentation(first, mode: "file", base: "branch-base")
    workspace.openFile("src/file.swift", base: "head")
    XCTAssertEqual(workspace.controller.allTabIds.count, 1)
    let restored = PaneWorkspace()
    try restored.restore(workspace.layout())
    XCTAssertEqual(
      restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.filePath, "src/file.swift")
    XCTAssertEqual(
      restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.fileMode, "file")
    XCTAssertEqual(
      restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.fileBase, "branch-base")
    let second = try XCTUnwrap(
      workspace.controller.splitPane(workspace.controller.allPaneIds[0], orientation: .vertical))
    workspace.controller.focusPane(second)
    workspace.openFile("src/file.swift", base: "head")
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    let secondTab = try XCTUnwrap(workspace.controller.selectedTab(inPane: second))
    XCTAssertNotEqual(secondTab.id, first)
    workspace.openFile("src/file.swift", base: "head")
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    XCTAssertEqual(workspace.controller.selectedTab(inPane: second)?.id, secondTab.id)
    XCTAssertEqual(
      workspace.controller.selectedTab(inPane: workspace.controller.allPaneIds[0])?.id, first)
  }

}

extension PaneWorkspaceTests {
  @MainActor func testTerminalCloseCanBeCancelledAndOnlyClosesAfterSuccess() async throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    let pane = try XCTUnwrap(workspace.controller.allPaneIds.first)
    workspace.open(.terminal, in: pane)
    workspace.open(.terminal, in: pane)
    let names = workspace.controller.tabs(inPane: pane).map(\.title)
    XCTAssertEqual(Set(names).count, 2)
    var cancelled = false
    workspace.requestClose = { _ in
      cancelled = true
      return false
    }
    workspace.closeCurrentTab()
    try await eventually { cancelled }
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    workspace.requestClose = { _ in true }
    workspace.closeCurrentTab()
    try await eventually { workspace.controller.allTabIds.count == 1 }
  }
}

extension PaneWorkspaceTests {
  @MainActor func testPaneCloseCancellationLeavesOnlyUnstoppedTabAndCanRetry() async throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    let pane = try XCTUnwrap(workspace.controller.allPaneIds.first)
    let other = try XCTUnwrap(workspace.controller.splitPane(pane, orientation: .vertical))
    workspace.open(.file, in: other)
    workspace.open(.terminal, in: pane)
    workspace.open(.terminal, in: pane)
    let terminals = workspace.controller.tabs(inPane: pane)
    var requests: [String] = []
    workspace.requestClose = { tab in
      requests.append(tab.id)
      return requests.count == 1
    }
    workspace.controller.closePane(pane)
    try await eventually { requests.count == 2 }
    XCTAssertTrue(workspace.controller.allPaneIds.contains(pane))
    XCTAssertEqual(workspace.controller.tabs(inPane: pane).map(\.id), [terminals[1].id])
    workspace.requestClose = { tab in
      requests.append(tab.id)
      return true
    }
    workspace.controller.closePane(pane)
    try await eventually { !workspace.controller.allPaneIds.contains(pane) }
    XCTAssertEqual(
      requests, terminals.map { $0.id.id.uuidString } + [terminals[1].id.id.uuidString])
    XCTAssertEqual(workspace.controller.allPaneIds, [other])
  }
}

extension PaneWorkspaceTests {
  @MainActor func testNodeTabsSelectExistingNodeRenameAndCloseWithoutStopping() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.openNode(.session, id: "session-node", title: "before")
    workspace.openNode(.command, id: "command-node", title: "command")
    workspace.openNode(.session, id: "session-node", title: "before")
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
    var snapshot = Releash_Client_V1_SessionTabSnapshot()
    snapshot.node.title = "after"
    workspace.updateSession(
      try XCTUnwrap(
        workspace.controller.selectedTab(inPane: workspace.controller.allPaneIds[0])?.id),
      snapshot: snapshot)
    let pane = try XCTUnwrap(workspace.controller.focusedPaneId)
    XCTAssertEqual(workspace.controller.selectedTab(inPane: pane)?.title, "after")
    var called = false
    workspace.requestClose = { _ in
      called = true
      return false
    }
    workspace.closeCurrentTab()
    XCTAssertFalse(called)
    XCTAssertEqual(try workspace.layout().tabs.map(\.nodeID), ["command-node"])
  }
}

extension PaneWorkspaceTests {
  @MainActor func testTerminalFocusFollowsSelectedTabAndFocusedPane() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    let pane = try XCTUnwrap(workspace.controller.allPaneIds.first)
    workspace.open(.terminal, in: pane)
    let terminal = try XCTUnwrap(workspace.controller.selectedTab(inPane: pane)?.id)
    workspace.openNode(.session, id: "session", title: "Session", in: pane)
    let session = try XCTUnwrap(workspace.controller.selectedTab(inPane: pane)?.id)
    XCTAssertFalse(workspace.isFocused(terminal, in: pane))
    XCTAssertTrue(workspace.isFocused(session, in: pane))
    workspace.controller.selectTab(terminal)
    XCTAssertTrue(workspace.isFocused(terminal, in: pane))
    XCTAssertFalse(workspace.isFocused(session, in: pane))
    let other = try XCTUnwrap(workspace.controller.splitPane(pane, orientation: .vertical))
    workspace.controller.focusPane(other)
    XCTAssertFalse(workspace.isFocused(terminal, in: pane))
  }
}
