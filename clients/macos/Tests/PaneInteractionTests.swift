import XCTest

@testable import Releash

final class PaneInteractionTests: XCTestCase {
  @MainActor func testClosingFocusedSplitRestoresHistoryAndCollapsesWithoutLosingOtherTabs()
    async throws
  {
    let model = try await testModel(TestHTTPClient())
    model.selectWorktree("/repo/tabs")
    try await eventually { model.layoutLoaded }
    let pane = model.layout.id
    for kind in PaneTab.Kind.allCases { model.openTab(kind, in: pane) }
    let tabs = model.layout.tabs
    model.selectTab(tabs[0].id, in: pane)
    model.selectTab(tabs[2].id, in: pane)
    model.closeTabs([tabs[2].id])
    XCTAssertEqual(model.layout.activeTab, tabs[0].id)
    model.moveTab(tabs[0].id, to: pane, edge: .right)
    XCTAssertNotEqual(model.focusedPane?.id, pane)
    model.closeTabs([tabs[0].id])
    XCTAssertEqual(model.layout.id, pane)
    XCTAssertEqual(model.focusedPane?.activeTab, tabs[1].id)
    try await eventually { model.pendingSaves["/repo/tabs"] == nil }
    await model.stop()
  }
  @MainActor func testSwitcherPreviewsWithoutChangingHistoryAndEscapeRestoresSelection()
    async throws
  {
    let model = try await testModel(TestHTTPClient())
    model.selectWorktree("/repo/switcher")
    try await eventually { model.layoutLoaded }
    let pane = model.layout.id
    for kind in PaneTab.Kind.allCases { model.openTab(kind, in: pane) }
    let tabs = model.layout.tabs
    model.selectTab(tabs[0].id, in: pane)
    let history = model.paneInteraction.history
    model.cycleRecentTab(1)
    XCTAssertEqual(model.layout.activeTab, tabs[2].id)
    XCTAssertEqual(model.paneInteraction.history, history)
    model.finishTabSwitcher(cancel: true)
    XCTAssertEqual(model.layout.activeTab, tabs[0].id)
    model.cycleRecentTab(1)
    model.finishTabSwitcher()
    XCTAssertEqual(model.layout.activeTab, tabs[2].id)
    XCTAssertEqual(model.paneInteraction.history.last, tabs[2].id)
    try await eventually { model.pendingSaves["/repo/switcher"] == nil }
    await model.stop()
  }
  func testOptionDragDuplicatesWithoutRemovingOriginalAcrossEverySplitDirection() {
    let original = PaneTab(kind: .terminal)
    let base = PaneLayout.pane(id: "pane", tabs: [original], active: original.id)
    for edge in [nil, PaneLayout.Edge.left, .right, .top, .bottom] {
      var layout = base
      let copy = layout.duplicate(original.id, to: "pane", edge: edge)
      XCTAssertNotNil(copy)
      XCTAssertNotEqual(copy, original.id)
      XCTAssertEqual(layout.tab(original.id), original)
      XCTAssertEqual(layout.tab(copy!)?.kind, original.kind)
      XCTAssertEqual(layout.panes.flatMap(\.tabs).count, 2)
      if edge != nil {
        XCTAssertEqual(layout.pane("pane")?.activeTab, original.id)
        XCTAssertNotEqual(layout.paneContaining(copy!)?.id, "pane")
      } else {
        XCTAssertEqual(layout.tabs.map(\.id), [original.id, copy!])
      }
    }
    var unchanged = base
    XCTAssertNil(unchanged.duplicate(original.id, to: "missing"))
    XCTAssertEqual(unchanged, base)
  }
  func testCloseUsesMostRecentlyActivatedSurvivingTab() {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    let pane = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[1].id)
    var interaction = PaneInteraction()
    interaction.activate(tabs[2].id)
    interaction.activate(tabs[0].id)
    interaction.activate(tabs[1].id)
    XCTAssertEqual(interaction.replacement(for: tabs[1].id, in: pane), tabs[0].id)
    interaction.activate("removed")
    XCTAssertEqual(interaction.replacement(for: tabs[1].id, in: pane), tabs[0].id)
    XCTAssertEqual(interaction.recentTabs(in: pane), [tabs[1].id, tabs[0].id, tabs[2].id])
  }
  func testHistoryDoesNotCrossPanesAndFallsBackLeft() {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    let pane = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[1].id)
    var interaction = PaneInteraction()
    interaction.activate("other-pane-tab")
    XCTAssertEqual(interaction.replacement(for: tabs[1].id, in: pane), tabs[0].id)
    XCTAssertEqual(interaction.replacement(for: tabs[0].id, in: pane), tabs[1].id)
    XCTAssertNil(
      interaction.replacement(
        for: tabs[0].id,
        in: .pane(id: "only", tabs: [tabs[0]], active: tabs[0].id)))
  }
  func testNewTabOpensImmediatelyAfterSelectedTab() {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    var pane = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[0].id)
    pane.open(.terminal, in: "pane")
    XCTAssertEqual(pane.tabs[0], tabs[0])
    XCTAssertEqual(Array(pane.tabs.suffix(2)), Array(tabs.suffix(2)))
    XCTAssertEqual(pane.tabs[1].id, pane.activeTab)
  }
}
