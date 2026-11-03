import Observation
import SwiftProtobuf
import XCTest

@testable import Releash

final class SessionHistoryRestoreTests: XCTestCase {
  @MainActor func testRestoreKeepsExistingTabsAndSelection() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo")
    try await eventually { model.layoutLoaded }
    model.panes.openSession(id: "agent-session", nodeID: "session-tree", title: "Session")
    model.panes.openNode(.command, id: "command", title: "Command")
    let layout = model.panes.layoutInput
    let changed = expectation(description: "Restore must preserve the current layout and selection")
    changed.isInverted = true
    withObservationTracking {
      _ = model.panes.layoutInput
    } onChange: {
      changed.fulfill()
    }
    var session = Releash_Client_V1_AgentSessionItemDto()
    session.id = "archived"

    SessionHistoryView(model: model, path: "/repo").restore(session)

    try await eventually { http.requests("RestoreAgentSession").count == 1 }
    await fulfillment(of: [changed], timeout: 0.2)
    XCTAssertEqual(model.panes.layoutInput, layout)
    let request = try Releash_Client_V1_RestoreAgentSessionRequest(
      serializedBytes: XCTUnwrap(http.requests("RestoreAgentSession").first?.message))
    XCTAssertEqual(request.agentSessionID, session.id)
    XCTAssertEqual(request.rows, UIStyle.terminalRows)
    XCTAssertEqual(request.cols, UIStyle.terminalColumns)
    XCTAssertFalse(request.callerRequestID.isEmpty)
    await model.stop()
  }
}
