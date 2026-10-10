import Connect
import Foundation
import SwiftProtobuf
import Synchronization
import XCTest

@testable import Releash

final class AppModelTests: XCTestCase {
  @MainActor func testDeletingSelectedWorktreeClearsLayoutWithoutChangingAnotherSelection() {
    let model = AppModel()
    model.selectedWorktree = "/repo/current"
    model.layoutLoaded = true
    model.layout.open(.terminal, in: model.layout.id)
    let layout = model.layout
    model.deselectWorktree("/repo/previous")
    XCTAssertEqual(model.selectedWorktree, "/repo/current")
    XCTAssertEqual(model.layout, layout)
    model.deselectWorktree("/repo/current")
    XCTAssertNil(model.selectedWorktree)
    XCTAssertFalse(model.layoutLoaded)
    guard case .pane(_, let tabs, let active) = model.layout else { return XCTFail("Missing empty pane") }
    XCTAssertTrue(tabs.isEmpty)
    XCTAssertNil(active)
  }
  @MainActor func testReselectingCurrentWorktreeKeepsLoadedLayout() {
    let model = AppModel()
    model.selectedWorktree = "/repo/current"
    model.layoutLoaded = true
    model.layout.open(.terminal, in: model.layout.id)
    let layout = model.layout
    model.selectWorktree("/repo/current")
    XCTAssertTrue(model.layoutLoaded)
    XCTAssertEqual(model.layout, layout)
  }
  @MainActor func testPendingLayoutInputSurvivesOlderServerEchoUntilSaveFinishes() throws {
    let model = AppModel()
    model.selectedWorktree = "/repo/worktree"
    let pane = model.layout.id
    model.layout.open(.terminal, in: pane)
    let input = model.layout
    var state = Releash_Client_V1_NullableWorkspaceStateDto()
    state.value.paneLayout = PaneLayout.empty().message
    model.pendingSaves["/repo/worktree"] = 2
    model.receiveWorkspaceState(state, path: "/repo/worktree")
    XCTAssertEqual(model.layout, input)
    model.pendingSaves["/repo/worktree"] = 1
    model.receiveWorkspaceState(state, path: "/repo/worktree")
    XCTAssertEqual(model.layout, input)
    model.pendingSaves.removeValue(forKey: "/repo/worktree")
    model.receiveWorkspaceState(state, path: "/repo/worktree")
    XCTAssertNotEqual(model.layout, input)
    XCTAssertTrue(model.layoutLoaded)
  }
  @MainActor func testSnapshotFromPreviouslySelectedWorktreeDoesNotReplaceCurrentLayout() throws {
    let model = AppModel()
    model.selectedWorktree = "/repo/current"
    let input = model.layout
    var state = Releash_Client_V1_NullableWorkspaceStateDto()
    state.value.paneLayout = PaneLayout.empty().message
    model.receiveWorkspaceState(state, path: "/repo/previous")
    XCTAssertEqual(model.layout, input)
    XCTAssertFalse(model.layoutLoaded)
  }
}

extension AppModelTests {
  @MainActor func testServerCommandErrorDetailsRemainVisible() async throws {
    let model = AppModel()
    var message = Releash_Client_V1_CommandError()
    message.message.value = "revspec 'missing-base' not found"
    var coded = Releash_Client_V1_CommandError()
    coded.coded.code = "WORKTREE_LOCKED"
    coded.coded.message = "Worktree is locked"
    for (detail, expected) in [(message, message.message.value), (coded, coded.coded.message)] {
      let error = ConnectError(
        code: .internalError, message: "Command failed",
        details: [.init(type: Releash_Client_V1_CommandError.protoMessageName,
                        payload: try detail.serializedData())])
      model.perform { throw error }
      try await eventually { model.failure == expected }
    }
    let malformed = ConnectError(
      code: .internalError, message: "Fallback reason",
      details: [.init(type: Releash_Client_V1_CommandError.protoMessageName,
                      payload: Data([0xff]))])
    model.perform { throw malformed }
    try await eventually { model.failure == "Fallback reason" }
  }

  @MainActor func testLocalFailureAndConnectFailureWithoutMessageRemainVisible() async throws {
    let model = AppModel()
    model.perform { throw CLIFailure(message: "local failure") }
    try await eventually { model.failure == "local failure" }
    let error = ConnectError(code: .internalError, message: nil)
    model.perform { throw error }
    try await eventually { model.failure == error.localizedDescription }
  }

  @MainActor func testLayoutsSavePerWorktreeAndRestoreOnReselectionAndNewClient() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var layouts: [String: PaneLayout] = [:]
    for (path, edge) in [("/repo/a", PaneLayout.Edge.right), ("/repo/b", .bottom)] {
      model.selectWorktree(path)
      try await eventually { model.layoutLoaded }
      let pane = model.layout.id
      model.editLayout { layout in
        for kind in PaneTab.Kind.allCases { layout.open(kind, in: pane) }
        if case .pane(_, let tabs, _) = layout { layout.move(tabs[0].id, to: pane, edge: edge) }
      }
      layouts[path] = model.layout
      try await eventually { model.pendingSaves[path] == nil }
    }
    let requests = http.requests("SaveWorkspaceState")
    XCTAssertEqual(requests.count, 2)
    for request in requests {
      let save = try Releash_Client_V1_SaveWorkspaceStateRequest(
        serializedBytes: request.message ?? Data())
      XCTAssertEqual(
        try PaneLayout(save.state.paneLayout),
        layouts[save.worktreeName])
    }
    for path in ["/repo/a", "/repo/b"] {
      model.selectWorktree(path)
      try await eventually { model.layoutLoaded && model.layout == layouts[path] }
    }
    await model.stop()
    let restarted = try await testModel(http)
    for path in ["/repo/a", "/repo/b"] {
      restarted.selectWorktree(path)
      try await eventually { restarted.layoutLoaded && restarted.layout == layouts[path] }
    }
    await restarted.stop()
  }

  @MainActor func testLayoutSaveFailureReachesOperationFailure() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded }
    http.state.withLock { $0.failedMethod = "SaveWorkspaceState" }
    let pane = model.layout.id
    model.editLayout { $0.open(.file, in: pane) }
    try await eventually {
      model.pendingSaves["/repo/a"] == nil
        && model.failure?.contains("SaveWorkspaceState failed") == true
    }
    XCTAssertNil(http.state.withLock { $0.layouts["/repo/a"] })
    try await eventually {
      if case .pane(_, let tabs, let active) = model.layout { return tabs.isEmpty && active == nil }
      return false
    }
    await model.stop()
  }
}

extension AppModelTests {
  @MainActor func testFailedSubscriptionCanBeRegisteredAgain() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try await eventually { http.requests("StartStateSubscription").count >= 3 }
    http.state.withLock { $0.failedMethod = "StartStateSubscription" }
    model.subscribe("repository-group-state", ["/retry"]) { _ in }
    try await eventually { model.failure?.contains("StartStateSubscription failed") == true }
    let count = http.requests("StartStateSubscription").count
    http.state.withLock { $0.failedMethod = nil }
    model.subscribe("repository-group-state", ["/retry"]) { _ in }
    try await eventually { http.requests("StartStateSubscription").count == count + 1 }
    let request = try Releash_Client_V1_StartStateSubscriptionRequest(
      serializedBytes: http.requests("StartStateSubscription").last!.message!)
    XCTAssertEqual(request.args, ["/retry"])
    await model.stop()
  }
  @MainActor func testRemovedRepositoryStopsSubscriptionAndClearsCollapsed() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    http.state.withLock {
      $0.repositories = [repository]
      $0.collapsed["/repo"] = true
    }
    let model = try await testModel(http)
    try await eventually { model.collapsed["/repo"] == true }
    let subscriptions = http.state.withLock { $0.subscriptions }
    let group = try XCTUnwrap(subscriptions.first { $0.target == "repository-group-state" })
    let workspaces = try XCTUnwrap(subscriptions.first { $0.target == "workspaces" })
    var event = Releash_Client_V1_StateSubscriptionEvent()
    event.subscriptionID = workspaces.subscriptionID
    event.snapshot.workspaces.repositories = .init()
    try TestHTTPClient.send(event, on: http.state.withLock { $0.streams.last! })
    try await eventually {
      model.repositories.isEmpty && model.collapsed["/repo"] == nil
        && !http.requests("StopStateSubscription").isEmpty
    }
    let stop = try Releash_Client_V1_StopStateSubscriptionRequest(
      serializedBytes: http.requests("StopStateSubscription").last!.message!)
    XCTAssertEqual(stop.subscriptionID, group.subscriptionID)
    await model.stop()
  }
  @MainActor func testCollapsedSavesOneRepositoryAndRestoresOnNewClient() async throws {
    let http = TestHTTPClient()
    http.state.withLock { storage in
      storage.repositories = ["/a", "/b"].map { path in
        var repo = Releash_Client_V1_WorkspaceRepositoryList()
        repo.path = path
        return repo
      }
    }
    let model = try await testModel(http)
    try await eventually { model.collapsed["/a"] == false && model.collapsed["/b"] == false }
    model.setCollapsed("/a", true)
    try await eventually { http.state.withLock { $0.collapsed["/a"] == true } }
    let save = try Releash_Client_V1_SaveRepositoryGroupStateRequest(
      serializedBytes: http.requests("SaveRepositoryGroupState")[0].message!)
    XCTAssertEqual(save.repositoryPath, "/a")
    XCTAssertTrue(save.collapsed)
    XCTAssertNil(http.state.withLock { $0.collapsed["/b"] })
    await model.stop()
    let restarted = try await testModel(http)
    try await eventually { restarted.collapsed["/a"] == true && restarted.collapsed["/b"] == false }
    await restarted.stop()
  }
  @MainActor func testStartFailureKeepsCLIGuidanceAndRetryDiscoversAndStartsAgain() async throws {
    let http = TestHTTPClient()
    let initial = try await testModel(http)
    await initial.stop()
    http.state.withLock {
      $0.requests.removeAll()
      $0.streams.removeAll()
      $0.subscriptions.removeAll()
    }
    let calls = Mutex<[String]>([])
    let fail = Mutex(true)
    let started = Mutex(false)
    let status = try JSONSerialization.data(withJSONObject: [
      "running": true, "compatibility": "compatible", "discovery_file": http.discoveryFile.path,
    ])
    let model = AppModel(
      cli: CLI(
        executable: URL(fileURLWithPath: "/test/releash"),
        run: { _, args in
          calls.withLock { $0.append(args.joined(separator: " ")) }
          if args == ["server", "start", "--json"] {
            if fail.withLock({ $0 }) {
              throw CLIFailure(
                message: #"{"error":{"message":"Cannot start server","guidance":"CLI startup guidance"}}"#)
            }
            started.withLock { $0 = true }
            return Data()
          }
          if started.withLock({ $0 }) {
            return status
          }
          return Data(#"{"running":false,"discovery_file":"unused"}"#.utf8)
        }), httpClient: http)
    await model.start()
    XCTAssertEqual(
      model.connectionFailure, "サーバを起動できません\nCannot start server\nCLI startup guidance")
    XCTAssertFalse(model.connected)
    fail.withLock { $0 = false }
    await model.start()
    try await eventually { model.connected && !http.requests("StartStateSubscription").isEmpty }
    XCTAssertNil(model.connectionFailure)
    XCTAssertEqual(
      calls.withLock { $0 },
      ["status --json", "server start --json", "status --json", "server start --json", "status --json"])
    await model.stop()
  }
}

extension AppModelTests {
  @MainActor func testDiscoveryAndCompatibilityFailuresPreserveCategoryAndCLIGuidance() async {
    for (status, expected) in [
      (#"{"running":false,"discovery_file":"unused","startup_guidance":"CLI startup guidance"}"#,
       "サーバを発見できません\nCLI startup guidance"),
      (#"{"running":true,"discovery_file":"unused","compatibility":"server_older","guidance":"CLI compatibility guidance"}"#,
       "サーバと互換性がありません\nserver_older\nCLI compatibility guidance")
    ] {
      let model = AppModel(cli: CLI(executable: URL(fileURLWithPath: "/test/releash"), run: { _, _ in
        Data(status.utf8)
      }))
      await model.start()
      XCTAssertEqual(model.connectionFailure, expected)
      XCTAssertFalse(model.connected)
      await model.stop()
    }
  }

  @MainActor func testMissingStartupGuidanceReportsInvalidCLIOutput() async {
    for status in [
      #"{"running":false,"discovery_file":"unused","startup_guidance":null}"#,
      #"{"running":false,"discovery_file":"unused"}"#,
    ] {
      let model = AppModel(cli: CLI(executable: URL(fileURLWithPath: "/test/releash"), run: { _, _ in
        Data(status.utf8)
      }))
      await model.start()
      XCTAssertEqual(
        model.connectionFailure,
        "サーバを発見できません\nCLI の status --json 出力に startup_guidance がありません\n\(status)")
      XCTAssertFalse(model.connected)
      await model.stop()
    }
  }

  @MainActor func testStatusCommandFailurePreservesCategoryAndStderr() async {
    let model = AppModel(cli: CLI(executable: URL(fileURLWithPath: "/test/releash"), run: { _, _ in
      throw CLIFailure(message: "status stderr")
    }))
    await model.start()
    XCTAssertEqual(model.connectionFailure, "サーバを発見できません\nstatus stderr")
    await model.stop()
  }
}
