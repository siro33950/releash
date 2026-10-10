import XCTest

@testable import Releash

final class PaneLayoutTests: XCTestCase {
  func testOpenMoveSplitCloseAndRestoreAllKinds() throws {
    for kind in PaneTab.Kind.allCases {
      var layout = PaneLayout.empty()
      let pane = layout.id
      layout.open(.file, in: pane)
      layout.open(kind, in: pane)
      guard case .pane(_, let tabs, _) = layout, let tab = tabs.last else {
        return XCTFail("Missing tab")
      }
      for edge in PaneLayout.Edge.allCases {
        var split = layout
        split.move(tab.id, to: pane, edge: edge)
        guard case .split(_, let axis, _, let first, let second) = split else {
          return XCTFail("Missing split")
        }
        XCTAssertEqual(axis, edge == .left || edge == .right ? .horizontal : .vertical)
        let moved = edge == .left || edge == .top ? first : second
        let original = edge == .left || edge == .top ? second : first
        XCTAssertEqual(original.id, pane)
        XCTAssertEqual(original, .pane(id: pane, tabs: [tabs[0]], active: tabs[0].id))
        XCTAssertEqual(moved, .pane(id: moved.id, tabs: [tab], active: tab.id))
        XCTAssertNotEqual(first.id, second.id)
        XCTAssertEqual(split.tab(tab.id)?.kind, kind)
        split.move(tab.id, to: pane)
        XCTAssertEqual(split, .pane(id: pane, tabs: [tabs[0], tab], active: tab.id))
        XCTAssertEqual(split.tab(tab.id)?.kind, kind)
        let restored = try PaneLayout(split.message)
        XCTAssertEqual(restored, split)
        split.close(tab.id)
        XCTAssertNil(split.tab(tab.id))
      }
    }
  }
  func testResizeRestoresRatioAndRejectsInvalidValues() throws {
    var layout = PaneLayout.empty()
    let pane = layout.id
    layout.open(.file, in: pane)
    layout.open(.terminal, in: pane)
    guard case .pane(_, let tabs, _) = layout, let tab = tabs.first else {
      return XCTFail("Missing tab")
    }
    layout.move(tab.id, to: pane, edge: .right)
    let split = layout.id
    layout.resize(split, ratio: 0.7)
    let restored = try PaneLayout(layout.message)
    guard case .split(_, _, let ratio, _, _) = restored else { return XCTFail("Missing split") }
    XCTAssertEqual(ratio, 0.7)
    for value in [0, 1, Double.nan] { layout.resize(split, ratio: value) }
    XCTAssertEqual(layout, restored)
  }
  func testUnknownDragLeavesLayoutUntouched() {
    var layout = PaneLayout.empty()
    let before = layout
    layout.move("missing", to: layout.id, edge: .left)
    XCTAssertEqual(layout, before)
  }
  func testMissingDestinationLeavesExistingTabUntouched() {
    var layout = PaneLayout.empty()
    layout.open(.terminal, in: layout.id)
    guard case .pane(_, let tabs, _) = layout else { return XCTFail("Missing pane") }
    let before = layout
    for edge in [nil, PaneLayout.Edge.left, .right, .top, .bottom] {
      layout.move(tabs[0].id, to: "missing", edge: edge)
      XCTAssertEqual(layout, before)
    }
  }
  func testActiveTabChangesWhenClosed() {
    var layout = PaneLayout.empty()
    let id = layout.id
    layout.open(.terminal, in: id)
    layout.open(.file, in: id)
    guard case .pane(_, let tabs, let active) = layout, let active else {
      return XCTFail("Missing active tab")
    }
    layout.close(active)
    guard case .pane(_, _, let next) = layout else { return XCTFail("Missing pane") }
    XCTAssertEqual(next, tabs.first?.id)
  }
  func testClosingMiddleTabSelectsRightNeighborAndKeepsInactiveSelection() {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    var layout = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[1].id)
    layout.close(tabs[1].id)
    XCTAssertEqual(layout, .pane(id: "pane", tabs: [tabs[0], tabs[2]], active: tabs[2].id))
    layout.close(tabs[0].id)
    XCTAssertEqual(layout, .pane(id: "pane", tabs: [tabs[2]], active: tabs[2].id))
  }
  func testTabReorderingPreservesIdentityAndRejectsMissingTarget() {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    var layout = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[0].id)
    layout.move(tabs[2].id, to: "pane", before: tabs[0].id)
    XCTAssertEqual(layout, .pane(id: "pane", tabs: [tabs[2], tabs[0], tabs[1]], active: tabs[2].id))
    let before = layout
    layout.move(tabs[0].id, to: "pane", before: "missing")
    XCTAssertEqual(layout, before)
    layout.move(tabs[2].id, to: "pane", before: tabs[2].id)
    XCTAssertEqual(layout, before)
    layout.move(tabs[2].id, to: "pane")
    XCTAssertEqual(layout, .pane(id: "pane", tabs: tabs, active: tabs[2].id))
  }
  func testClosingLastTabCollapsesNestedSplitAndPreservesSibling() throws {
    let a = PaneTab(kind: .terminal)
    let b = PaneTab(kind: .workflow)
    let c = PaneTab(kind: .file)
    let left = PaneLayout.pane(id: "a", tabs: [a], active: a.id)
    let bottom = PaneLayout.pane(id: "c", tabs: [c], active: c.id)
    var layout = PaneLayout.split(
      id: "outer", axis: .horizontal, ratio: 0.3, first: left,
      second: .split(id: "inner", axis: .vertical, ratio: 0.6,
        first: .pane(id: "b", tabs: [b], active: b.id), second: bottom))
    layout.close(b.id)
    XCTAssertEqual(layout, .split(id: "outer", axis: .horizontal, ratio: 0.3,
                                 first: left, second: bottom))
    XCTAssertEqual(try PaneLayout(layout.message), layout)
    layout.close(a.id)
    XCTAssertEqual(layout, bottom)
    layout.close(c.id)
    XCTAssertEqual(layout, .pane(id: "c", tabs: [], active: nil))
    layout.open(.terminal, in: "c")
    guard case .pane(_, let tabs, _) = layout else { return XCTFail("Missing last pane") }
    XCTAssertEqual(tabs.count, 1)
  }
  func testMovingOnlyTabWithinItsPaneDoesNotCreateEmptySplit() {
    for edge in [nil, PaneLayout.Edge.left, .right, .top, .bottom] {
      let tab = PaneTab(kind: .terminal)
      var layout = PaneLayout.pane(id: "pane", tabs: [tab], active: tab.id)
      let before = layout
      layout.move(tab.id, to: "pane", edge: edge)
      XCTAssertEqual(layout, before)
    }
  }
  func testNestedSplitMinimumKeepsEveryPaneVisible() {
    let layout = PaneLayout.split(id: "outer", axis: .horizontal, ratio: 0.5,
      first: .empty(),
      second: .split(id: "inner", axis: .vertical, ratio: 0.5,
                     first: .empty(), second: .empty()))
    XCTAssertEqual(layout.minimumLength(along: .horizontal), UIStyle.minimumPaneSize * 2 + UIStyle.spacing)
    XCTAssertEqual(layout.minimumLength(along: .vertical), UIStyle.minimumPaneSize * 2 + UIStyle.spacing)
  }
  func testMovingLastTabToSiblingEdgePreservesBothTabs() throws {
    for edge in PaneLayout.Edge.allCases {
      let a = PaneTab(kind: .terminal)
      let b = PaneTab(kind: .file)
      var layout = PaneLayout.split(id: "root", axis: .horizontal, ratio: 0.4,
        first: .pane(id: "a", tabs: [a], active: a.id),
        second: .pane(id: "b", tabs: [b], active: b.id))
      layout.move(a.id, to: "b", edge: edge)
      guard case .split(_, let axis, _, let first, let second) = layout else {
        return XCTFail("Missing destination split")
      }
      let moved = edge == .left || edge == .top ? first : second
      let target = edge == .left || edge == .top ? second : first
      XCTAssertEqual(axis, edge == .left || edge == .right ? .horizontal : .vertical)
      XCTAssertEqual(moved, .pane(id: moved.id, tabs: [a], active: a.id))
      XCTAssertEqual(target, .pane(id: "b", tabs: [b], active: b.id))
      XCTAssertFalse(layout.containsPane("a"))
      XCTAssertEqual(try PaneLayout(layout.message), layout)
    }
  }
}
