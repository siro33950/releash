import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class AppModelTests: XCTestCase {
  @MainActor func testPendingLayoutInputSurvivesOlderServerEchoUntilSaveFinishes() throws {
    let model = AppModel()
    model.selectedWorktree = "/repo/worktree"
    let pane = model.layout.id
    model.layout.open(.terminal, in: pane)
    let input = model.layout
    var state = Releash_Client_V1_NullableWorkspaceStateDto()
    state.value.paneLayoutJson = String(
      decoding: try JSONEncoder().encode(PaneLayout.empty()), as: UTF8.self)
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
    state.value.paneLayoutJson = String(
      decoding: try JSONEncoder().encode(PaneLayout.empty()), as: UTF8.self)
    model.receiveWorkspaceState(state, path: "/repo/previous")
    XCTAssertEqual(model.layout, input)
    XCTAssertFalse(model.layoutLoaded)
  }
}

extension AppModelTests {
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
        try JSONDecoder().decode(PaneLayout.self, from: Data(save.state.paneLayoutJson.utf8)),
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
    await model.stop()
  }
}
