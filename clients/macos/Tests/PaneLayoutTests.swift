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
        guard case .split(_, _, _, let movedFirst, let movedSecond) = split else {
          return XCTFail("Missing split after move")
        }
        let destination = movedFirst.id == pane ? movedFirst : movedSecond
        let source = movedFirst.id == moved.id ? movedFirst : movedSecond
        XCTAssertEqual(destination, .pane(id: pane, tabs: [tabs[0], tab], active: tab.id))
        XCTAssertEqual(source, .pane(id: moved.id, tabs: [], active: nil))
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
}
