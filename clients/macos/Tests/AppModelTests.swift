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
    model.layout = testLayout(.terminal)
    let layout = model.layout
    model.deselectWorktree("/repo/previous")
    XCTAssertEqual(model.selectedWorktree, "/repo/current")
    XCTAssertEqual(model.layout, layout)
    model.deselectWorktree("/repo/current")
    XCTAssertNil(model.selectedWorktree)
    XCTAssertFalse(model.layoutLoaded)
    guard case .pane(_, let tabs, let active) = model.layout else {
      return XCTFail("Missing empty pane")
    }
    XCTAssertTrue(tabs.isEmpty)
    XCTAssertNil(active)
  }
  @MainActor func testReselectingCurrentWorktreeKeepsLoadedLayout() {
    let model = AppModel()
    model.selectedWorktree = "/repo/current"
    model.layoutLoaded = true
    model.layout = testLayout(.terminal)
    let layout = model.layout
    model.selectWorktree("/repo/current")
    XCTAssertTrue(model.layoutLoaded)
    XCTAssertEqual(model.layout, layout)
  }
  @MainActor func testPendingLayoutInputSurvivesOlderServerEchoUntilSaveFinishes() throws {
    let model = AppModel()
    model.selectedWorktree = "/repo/worktree"
    model.layout = testLayout(.terminal)
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
        details: [
          .init(
            type: Releash_Client_V1_CommandError.protoMessageName,
            payload: try detail.serializedData())
        ])
      model.perform { throw error }
      try await eventually { model.failure == expected }
    }
    let malformed = ConnectError(
      code: .internalError, message: "Fallback reason",
      details: [
        .init(
          type: Releash_Client_V1_CommandError.protoMessageName,
          payload: Data([0xff]))
      ])
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
    for (path, axis) in [("/repo/a", PaneLayout.Axis.horizontal), ("/repo/b", .vertical)] {
      model.selectWorktree(path)
      try await eventually { model.layoutLoaded }
      let tabs = PaneTab.Kind.allCases.map { PaneTab(kind: $0) }
      model.editLayout { layout in
        layout = .split(
          id: "split", axis: axis, ratio: 0.5,
          first: .pane(id: "left", tabs: Array(tabs.dropFirst()), active: tabs.last!.id),
          second: .pane(id: "right", tabs: [tabs[0]], active: tabs[0].id))
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
    model.editLayout { $0 = testLayout(.file) }
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
                message:
                  #"{"error":{"message":"Cannot start server","guidance":"CLI startup guidance"}}"#)
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
      [
        "status --json", "server start --json", "status --json", "server start --json",
        "status --json",
      ])
    await model.stop()
  }
}

extension AppModelTests {
  @MainActor func testDiscoveryAndCompatibilityFailuresPreserveCategoryAndCLIGuidance() async {
    for (status, expected) in [
      (
        #"{"running":false,"discovery_file":"unused","startup_guidance":"CLI startup guidance"}"#,
        "サーバを発見できません\nCLI startup guidance"
      ),
      (
        #"{"running":true,"discovery_file":"unused","compatibility":"server_older","guidance":"CLI compatibility guidance"}"#,
        "サーバと互換性がありません\nserver_older\nCLI compatibility guidance"
      ),
    ] {
      let model = AppModel(
        cli: CLI(
          executable: URL(fileURLWithPath: "/test/releash"),
          run: { _, _ in
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
      let model = AppModel(
        cli: CLI(
          executable: URL(fileURLWithPath: "/test/releash"),
          run: { _, _ in
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
    let model = AppModel(
      cli: CLI(
        executable: URL(fileURLWithPath: "/test/releash"),
        run: { _, _ in
          throw CLIFailure(message: "status stderr")
        }))
    await model.start()
    XCTAssertEqual(model.connectionFailure, "サーバを発見できません\nstatus stderr")
    await model.stop()
  }
}

private func testLayout(_ kind: PaneTab.Kind) -> PaneLayout {
  let tab = PaneTab(kind: kind)
  return .pane(id: "test-pane", tabs: [tab], active: tab.id)
}

extension AppModelTests {
  @MainActor func testSerialSavesIgnoreOldEchoAndRestoreLastOfPendingEdits() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let path = "/repo/a"
    model.selectWorktree(path)
    try await eventually { model.layoutLoaded }
    let subscription = try XCTUnwrap(http.state.withLock {
      $0.subscriptions.last { $0.target == "workspace-state" }
    })
    http.state.withLock { $0.deferSave = true }
    model.editLayout { $0 = testLayout(.terminal) }
    try await eventually { http.state.withLock { $0.pendingSave != nil } }
    model.editLayout { $0 = testLayout(.file) }
    model.editLayout { $0 = testLayout(.workflow) }
    let last = model.layout
    XCTAssertEqual(http.requests("SaveWorkspaceState").count, 1)
    XCTAssertEqual(model.pendingSaves[path], 3)
    let oldLayout = PaneLayout.empty()
    var receivedEcho = false
    let echoSubscriber = model.subscribe("workspace-state", [path, path]) { payload in
      receivedEcho = payload.workspaceState.hasValue
        && payload.workspaceState.value.paneLayout == oldLayout.message
    }
    var echo = Releash_Client_V1_StateSubscriptionEvent()
    echo.subscriptionID = subscription.subscriptionID
    echo.snapshot.workspaceState.value.paneLayout = oldLayout.message
    try TestHTTPClient.send(echo, on: http.state.withLock { $0.streams.last! })
    try await eventually { receivedEcho }
    XCTAssertEqual(model.pendingSaves[path], 3)
    XCTAssertEqual(model.layout, last)
    model.unsubscribe(echoSubscriber)
    let release = try XCTUnwrap(http.state.withLock { state in
      let callback = state.pendingSave
      state.pendingSave = nil
      state.deferSave = false
      return callback
    })
    release(HTTPResponse(code: .ok, headers: ["content-type": ["application/proto"]], message: Data(), trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    try await eventually { model.pendingSaves[path] == nil }
    let saves = try http.requests("SaveWorkspaceState").map {
      try Releash_Client_V1_SaveWorkspaceStateRequest(serializedBytes: $0.message!)
    }
    XCTAssertEqual(saves.count, 3)
    for (request, kind) in zip(saves, [PaneTab.Kind.terminal, .file, .workflow]) {
      guard case .pane(_, let tabs, _) = try PaneLayout(request.state.paneLayout) else {
        return XCTFail("Saved pane missing")
      }
      XCTAssertEqual(tabs.first?.kind, kind)
    }
    XCTAssertEqual(try PaneLayout(saves.last!.state.paneLayout), last)
    model.selectWorktree("/repo/b")
    try await eventually { model.layoutLoaded }
    model.selectWorktree(path)
    try await eventually { model.layoutLoaded && model.layout == last }
    await model.stop()
    let restarted = try await testModel(http)
    restarted.selectWorktree(path)
    try await eventually { restarted.layoutLoaded && restarted.layout == last }
    await restarted.stop()
  }

  @MainActor func testSharedSubscriptionDeliversCacheAndStopsOnlyLastSubscriber() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    try await eventually { http.requests("StartStateSubscription").count >= 3 }
    let initial = http.requests("StartStateSubscription").count
    var first: [Bool] = []
    var second: [Bool] = []
    let one = model.subscribe("repository-group-state", ["/shared"]) {
      first.append($0.repositoryGroupState.collapsed)
    }
    try await eventually { first == [false] }
    let two = model.subscribe("repository-group-state", ["/shared"]) {
      second.append($0.repositoryGroupState.collapsed)
    }
    XCTAssertEqual(second, [false])
    XCTAssertEqual(http.requests("StartStateSubscription").count, initial + 1)
    let subscription = try XCTUnwrap(http.state.withLock { $0.subscriptions.last })
    var event = Releash_Client_V1_StateSubscriptionEvent()
    event.subscriptionID = subscription.subscriptionID
    event.change.payload.repositoryGroupState.collapsed = true
    try TestHTTPClient.send(event, on: http.state.withLock { $0.streams.last! })
    try await eventually { first == [false, true] && second == [false, true] }
    model.unsubscribe(one)
    XCTAssertTrue(http.requests("StopStateSubscription").isEmpty)
    event.change.payload.repositoryGroupState.collapsed = false
    try TestHTTPClient.send(event, on: http.state.withLock { $0.streams.last! })
    try await eventually { second == [false, true, false] }
    XCTAssertEqual(first, [false, true])
    model.unsubscribe(two)
    try await eventually { http.requests("StopStateSubscription").count == 1 }
    var third: [Bool] = []
    let three = model.subscribe("repository-group-state", ["/shared"]) {
      third.append($0.repositoryGroupState.collapsed)
    }
    XCTAssertTrue(third.isEmpty)
    try await eventually { third == [false] }
    model.unsubscribe(three)
    await model.stop()
  }
}
