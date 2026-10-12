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
}
