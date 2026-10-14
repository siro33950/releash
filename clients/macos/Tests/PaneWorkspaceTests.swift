import Bonsplit
import XCTest

@testable import Releash

final class PaneWorkspaceTests: XCTestCase {
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
    try restored.restore(PaneLayout(workspace.layout().message))
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
    workspace.open(.workflow, in: second)
    XCTAssertEqual(try workspace.layout().tabs.map(\.kind), [.workflow])
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
  @MainActor func testFileTabsReuseFocusedPaneAndPersistPresentation() throws {
    let workspace = PaneWorkspace()
    try workspace.restore(.empty())
    workspace.openFile("src/file.swift", base: "head")
    let first = try XCTUnwrap(workspace.controller.allTabIds.first)
    workspace.setFilePresentation(first, mode: "file", base: "branch-base")
    workspace.openFile("src/file.swift", base: "head")
    XCTAssertEqual(workspace.controller.allTabIds.count, 1)
    let restored = PaneWorkspace()
    try restored.restore(PaneLayout(workspace.layout().message))
    XCTAssertEqual(restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.filePath, "src/file.swift")
    XCTAssertEqual(restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.fileMode, "file")
    XCTAssertEqual(restored.tab(try XCTUnwrap(restored.controller.allTabIds.first))?.fileBase, "branch-base")
    let second = try XCTUnwrap(workspace.controller.splitPane(workspace.controller.allPaneIds[0], orientation: .vertical))
    workspace.controller.focusPane(second)
    workspace.openFile("src/file.swift", base: "head")
    XCTAssertEqual(workspace.controller.allTabIds.count, 2)
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
    workspace.requestClose = { _ in cancelled = true; return false }
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
    workspace.requestClose = { tab in requests.append(tab.id); return true }
    workspace.controller.closePane(pane)
    try await eventually { !workspace.controller.allPaneIds.contains(pane) }
    XCTAssertEqual(requests, terminals.map { $0.id.id.uuidString } + [terminals[1].id.id.uuidString])
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
    workspace.renameNode("session-node", title: "after")
    let pane = try XCTUnwrap(workspace.controller.focusedPaneId)
    XCTAssertEqual(workspace.controller.selectedTab(inPane: pane)?.title, "after")
    var called = false
    workspace.requestClose = { _ in called = true; return false }
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
