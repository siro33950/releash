import XCTest

@testable import Releash

final class PaneLayoutTests: XCTestCase {
  func testWireRoundTripPreservesKindsOrderSelectionAndNestedSplits() throws {
    let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
    let layout = PaneLayout.split(
      id: "outer", axis: .horizontal, ratio: 0.3,
      first: .pane(id: "first", tabs: tabs, active: tabs[1].id),
      second: .split(
        id: "inner", axis: .vertical, ratio: 0.7,
        first: .empty(), second: .empty()))
    XCTAssertEqual(try PaneLayout(layout.message), layout)
  }

  func testUnknownTabAndSplitKindsAndMissingNodesAreRejected() {
    var tab = Releash_Client_V1_PaneTab()
    tab.id = "unknown"
    tab.kind = .UNRECOGNIZED(100)
    var wire = Releash_Client_V1_PaneLayout()
    wire.pane.id = "pane"
    wire.pane.tabs = [tab]
    XCTAssertThrowsError(try PaneLayout(wire))
    wire = .init()
    wire.split.axis = .UNRECOGNIZED(100)
    XCTAssertThrowsError(try PaneLayout(wire))
    wire.split.axis = .horizontal
    XCTAssertThrowsError(try PaneLayout(wire))
    XCTAssertThrowsError(try PaneLayout(.init()))
  }
}

extension PaneLayoutTests {
  func testSessionAndCommandNodeReferencesSurviveWireRoundTrip() throws {
    let tabs = [
      PaneTab(kind: .session, agentSessionID: "agent-session"),
      PaneTab(kind: .command, nodeID: "command-node"),
    ]
    let layout = PaneLayout.pane(id: "pane", tabs: tabs, active: tabs[1].id)
    XCTAssertEqual(try PaneLayout(layout.message), layout)
  }
}
