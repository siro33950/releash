import AppKit
import SwiftProtobuf
import XCTest

@testable import Releash

final class SessionWorkflowMenuTests: XCTestCase {
  @MainActor func test_NodeのWorkflowメニュー_Root配信待ちでは一覧を表示せず無効になる() async throws {
    // Given
    let model = AppModel()
    model.workflows = [.with { $0.name = "pending-flow" }]
    model.pendingRoots["/repo/a"] = "pending-root"
    let host = await ViewHarness(SessionWorkflowMenu(model: model, path: "/repo/a"))
    defer { host.close() }
    // When
    try await MenuHarness().open(
      press: { try await host.press("開始") },
      inspect: { menu in
        // Then
        let workflow = try XCTUnwrap(menu.items.first { $0.title == "Workflow" })
        XCTAssertFalse(workflow.isEnabled)
        XCTAssertNil(workflow.submenu)
      })
  }

  @MainActor func test_Sessionメニュー_provider一覧と選択をpane付きで送る() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded }
    model.providers = [.with { $0.value = .codex }, .with { $0.value = .claude }]
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.first)
    let host = await ViewHarness(PaneOpenMenu(model: model, pane: pane))
    defer { host.close() }
    for (index, provider) in model.providers.enumerated() {
      // When
      try await MenuHarness().open(
        selection: ["Session", provider.displayName], press: { try await host.press("開く") },
        inspect: { menu in
          let submenu = try XCTUnwrap(menu.items.first { $0.title == "Session" }?.submenu)
          submenu.update()
          XCTAssertEqual(submenu.items.map(\.title), model.providers.map(\.displayName))
        })
      // Then
      try await eventually {
        http.requests("CreateAgentSession").count == index + 1
      }
      let request = try Releash_Client_V1_CreateAgentSessionRequest(
        serializedBytes: XCTUnwrap(http.requests("CreateAgentSession").last?.message))
      XCTAssertEqual(request.provider, provider.wireName)
      XCTAssertEqual(request.worktreePath, "/repo/a")
      XCTAssertEqual(request.paneID, model.panes.paneIdentifier(pane))
    }
    await model.stop()
  }

  @MainActor func test_Terminalメニュー_worktree選択の有無によらず押したpaneに開く() async throws {
    for path in [String?.none, "/repo/a"] {
      // Given
      let model = AppModel()
      model.selectedWorktree = path
      model.layoutLoaded = true
      try model.panes.restore(
        .split(
          id: "split", axis: .horizontal, ratio: 0.5,
          first: .pane(id: "first", tabs: [], active: nil),
          second: .pane(id: "second", tabs: [], active: nil)))
      let first = try XCTUnwrap(model.panes.controller.allPaneIds.first)
      let second = try XCTUnwrap(model.panes.controller.allPaneIds.last)
      model.panes.controller.focusPane(first)
      let host = await ViewHarness(PaneOpenMenu(model: model, pane: second))
      defer { host.close() }
      // When
      try await MenuHarness().open(
        selection: ["Terminal"], press: { try await host.press("開く") },
        inspect: { menu in
          XCTAssertEqual(
            menu.items.map(\.title),
            path == nil ? ["Terminal"] : ["Session", "Workflow", "Terminal"])
        })
      // Then
      try await eventually { model.panes.controller.tabs(inPane: second).count == 1 }
      XCTAssertTrue(model.panes.controller.tabs(inPane: first).isEmpty)
      let selected = try XCTUnwrap(model.panes.controller.selectedTab(inPane: second))
      XCTAssertEqual(model.panes.tab(selected.id)?.kind, .terminal)
    }
  }
}
