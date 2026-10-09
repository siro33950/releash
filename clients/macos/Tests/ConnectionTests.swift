import Connect
import XCTest

@testable import Releash

final class ConnectionTests: XCTestCase {
  @MainActor func testRequestsAuthenticateAndRecoveryResubscribesUpdatedSidebarAndLayout()
    async throws
  {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    http.state.withLock { $0.repositories = [repository] }
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded && !http.requests("StartStateSubscription").isEmpty }
    model.refresh()
    try await eventually { !http.requests("RefreshWorkspaces").isEmpty }
    let previousIDs = http.state.withLock { $0.subscriptions.map(\.subscriptionID) }
    var updated = PaneLayout.empty()
    updated.open(.workflow, in: updated.id)
    var state = Releash_Client_V1_WorkspaceStateDto()
    state.paneLayoutJson = String(decoding: try JSONEncoder().encode(updated), as: UTF8.self)
    repository.status.error = "read failed while disconnected"
    let stream = http.state.withLock { storage in
      storage.repositories = [repository]
      storage.layouts["/repo/a"] = state
      return storage.streams[0]
    }
    stream.receiveClose(
      .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    try await eventually { model.reconnecting }
    try await eventually {
      !model.reconnecting && model.layout == updated
        && model.repositories.first?.status.error == "read failed while disconnected"
    }
    let subscriptions = http.state.withLock { $0.subscriptions }
    XCTAssertTrue(
      subscriptions.contains {
        $0.target == "workspaces" && !previousIDs.contains($0.subscriptionID)
      })
    XCTAssertTrue(
      subscriptions.contains {
        $0.target == "workspace-state" && $0.args == ["/repo/a", "/repo/a"]
          && !previousIDs.contains($0.subscriptionID)
      })
    let requests = http.state.withLock { $0.requests }
    XCTAssertGreaterThanOrEqual(
      requests.filter { $0.url.lastPathComponent == "OpenStateStream" }.count, 2)
    for request in requests {
      XCTAssertEqual(request.headers["authorization"], ["Bearer operator-test"])
      XCTAssertFalse(request.headers.keys.contains { $0.lowercased() == "origin" })
    }
    await model.stop()
  }
}
