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
    state.paneLayout = updated.message
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

extension ConnectionTests {
  @MainActor func testRecoveryClearsLayoutWhenSnapshotHasNoLayoutOrNoState() async throws {
    for hasState in [true, false] {
      let http = TestHTTPClient()
      let model = try await testModel(http)
      model.selectWorktree("/repo/a")
      try await eventually { model.layoutLoaded }
      model.editLayout { $0.open(.terminal, in: $0.id) }
      try await eventually { model.pendingSaves["/repo/a"] == nil }
      XCTAssertNotNil(
        model.layout.tab(
          {
            if case .pane(_, let tabs, _) = model.layout { return tabs[0].id }
            return ""
          }()))
      let stream = http.state.withLock { storage in
        storage.layouts["/repo/a"] = hasState ? Releash_Client_V1_WorkspaceStateDto() : nil
        return storage.streams.last!
      }
      stream.receiveClose(
        .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
      try await eventually { model.reconnecting }
      try await eventually {
        if case .pane(_, let tabs, let active) = model.layout {
          return !model.reconnecting && tabs.isEmpty && active == nil
        }
        return false
      }
      await model.stop()
    }
  }
}

extension ConnectionTests {
  @MainActor func testRecoveryRetriesFailedRegistrationWithoutRevivingUnsubscribedTargets()
    async throws
  {
    for target in ["workspaces", "workspace-state"] {
      let http = TestHTTPClient()
      let model = try await testModel(http)
      model.selectWorktree("/repo/a")
      model.subscribe("removed", ["/old"]) { _ in }
      try await eventually {
        model.layoutLoaded
          && http.state.withLock { $0.subscriptions.contains { $0.target == "removed" } }
      }
      model.unsubscribe("removed", ["/old"])
      let removedCount = http.state.withLock {
        $0.subscriptions.filter { $0.target == "removed" }.count
      }
      var updated = PaneLayout.empty()
      updated.open(.workflow, in: updated.id)
      var state = Releash_Client_V1_WorkspaceStateDto()
      state.paneLayout = updated.message
      var repository = Releash_Client_V1_WorkspaceRepositoryList()
      repository.path = "/updated"
      let stream = http.state.withLock { storage in
        storage.layouts["/repo/a"] = state
        storage.repositories = [repository]
        storage.failedSubscriptionTargets[target] = 1
        return storage.streams.last!
      }
      stream.receiveClose(
        .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
      try await eventually {
        http.state.withLock { $0.streams.count >= 3 && $0.failedSubscriptionTargets[target] == 0 }
          && model.layout == updated && model.repositories.first?.path == "/updated"
          && !model.reconnecting
      }
      XCTAssertEqual(
        http.state.withLock { $0.subscriptions.filter { $0.target == "removed" }.count },
        removedCount)
      let subscription = http.state.withLock {
        $0.subscriptions.last { $0.target == "workspace-state" }!
      }
      var notification = Releash_Client_V1_StateSubscriptionEvent()
      notification.subscriptionID = subscription.subscriptionID
      notification.change.payload.workspaceState = .init()
      try TestHTTPClient.send(notification, on: http.state.withLock { $0.streams.last! })
      try await eventually {
        if case .pane(_, let tabs, _) = model.layout { return tabs.isEmpty }
        return false
      }
      await model.stop()
    }
  }
}
