import AppKit
import ApplicationServices
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class PaneOpenAcceptanceTests: XCTestCase {

  @MainActor func test_B001_中央メニューの項目と順序() async throws {
    // Given
    let (_, model) = try await setup()
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.last)
    let host = await ViewHarness(PaneOpenMenu(model: model, pane: pane))
    defer { host.close() }
    // When / Then
    try await menu(host, button: "開く") {
      XCTAssertEqual($0.items.map(\.title), ["Session", "Workflow", "Terminal"])
    }
    await model.stop()
  }

  @MainActor func test_B003_Nodeメニューの項目と順序() async throws {
    // Given
    let (_, model) = try await setup()
    let host = await ViewHarness(RootListView(model: model, path: "/repo/a"))
    defer { host.close() }
    // When / Then
    try await menu(host, button: "開始") {
      XCTAssertEqual($0.items.map(\.title), ["Session", "Workflow"])
    }
    await model.stop()
  }

  @MainActor func test_B002_中央とNodeの開始メニューの一覧が一致する() async throws {
    // Given
    let (http, model) = try await setup()
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.last)
    let central = await ViewHarness(PaneOpenMenu(model: model, pane: pane))
    defer { central.close() }
    var centralLists: [[String]] = []
    // When / Then
    try await menu(central, button: "開く") { menu in
      centralLists = try ["Session", "Workflow"].map { title in
        try XCTUnwrap(menu.items.first { $0.title == title }?.submenu).items.map(\.title)
      }
    }
    let node = await ViewHarness(RootListView(model: model, path: "/repo/a"))
    defer { node.close() }
    try await menu(node, button: "開始") { menu in
      let lists = try ["Session", "Workflow"].map { title in
        try XCTUnwrap(menu.items.first { $0.title == title }?.submenu).items.map(\.title)
      }
      XCTAssertEqual(centralLists, lists)
      XCTAssertEqual(
        lists, [model.providers.map(\.displayName), ["acceptance-flow", "other-flow"]])
    }
    XCTAssertTrue(http.requests("StartWorkflow").isEmpty)
    await model.stop()
  }

  @MainActor func test_B004_Sessionは押したpaneで選択される() async throws {
    // Given
    let (http, model) = try await setup()
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    // When
    try await menu(host, button: "開く", selection: ["Session", "Codex"], rightmost: true)
    try await eventually { !http.requests("CreateAgentSession").isEmpty }
    // Then
    let request = try Releash_Client_V1_CreateAgentSessionRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateAgentSession").last?.message))
    XCTAssertEqual(request.provider, "codex")
    XCTAssertEqual(request.workspaceIdentity, "/repo/a")
    XCTAssertEqual(request.worktreePath, "/repo/a")
    XCTAssertEqual(request.paneID, "second")
    try await eventually { model.pendingSaves["/repo/a"] == nil }
    var state = Releash_Client_V1_StatePayload()
    state.workspaceState.value.version = 1
    state.workspaceState.value.paneLayout =
      PaneLayout.split(
        id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: "first", tabs: [], active: nil),
        second: .pane(
          id: "second",
          tabs: [
            PaneTab(id: "session-agent-session", kind: .session, agentSessionID: "agent-session")
          ], active: nil)
      ).message
    state.workspaceState.value.etag = "1"
    http.state.withLock { $0.layouts["/repo/a"] = state.workspaceState.value }
    try http.publish(state, target: "workspace-state")
    try await eventuallyView(host) {
      host.element("terminal:agent-session") != nil && host.element("Accepted Session") != nil
    }
    let tab = try XCTUnwrap(PaneOpenScreen.elements(host, named: "Accepted Session").first)
    let terminal = try XCTUnwrap(
      PaneOpenScreen.elements(host, named: "terminal:agent-session").first)
    XCTAssertGreaterThan(try PaneOpenScreen.x(tab), host.window.frame.midX)
    XCTAssertGreaterThanOrEqual(try PaneOpenScreen.x(terminal), host.window.frame.midX)
    await model.stop()
  }

  @MainActor func test_B005_Workflow開始後も右カラムを維持してNodeに実行木を表示する() async throws {
    // Given
    let (http, model) = try await setup()
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.last)
    let host = await ViewHarness(
      HStack {
        PaneOpenMenu(model: model, pane: pane)
        RightPanelView(expanded: .constant(true), model: model)
      })
    defer { host.close() }
    try await host.press("right-panel:threads")
    try await eventuallyView(host) { host.element("thread:retained-thread") != nil }
    // When
    try await menu(host, button: "開く", selection: ["Workflow", "acceptance-flow"])
    try await eventually { !http.requests("StartWorkflow").isEmpty }
    // Then
    let request = try Releash_Client_V1_StartWorkflowRequest(
      serializedBytes: XCTUnwrap(http.requests("StartWorkflow").last?.message))
    XCTAssertEqual(request.workflowName, "acceptance-flow")
    XCTAssertEqual(request.worktreePath, "/repo/a")
    try deliverRoot(http)
    try await eventuallyView(host) {
      host.element("thread:retained-thread") != nil
        && host.element("thread-body:retained-thread") != nil
    }
    try await host.press("right-panel:roots")
    try await eventuallyView(host) { host.element("root:workflow-execution") != nil }
    host.close()
    await model.stop()
  }

  @MainActor func test_B006_WorkflowのSessionタブはNodeから開始した場合と同じpaneに開く() async throws {
    var destinations: [Bool] = []
    for central in [false, true] {
      // Given
      let (http, model) = try await setup()
      let host = await ViewHarness(
        VStack {
          RootListView(model: model, path: "/repo/a").frame(height: 140)
          WorkspacePaneView(model: model)
        })
      // When
      try await menu(
        host, button: central ? "開く" : "開始", selection: ["Workflow", "acceptance-flow"],
        rightmost: central)
      try await eventually { !http.requests("StartWorkflow").isEmpty }
      try deliverRoot(http)
      // Then
      try await eventuallyView(host) {
        host.element("terminal:workflow-session") != nil && host.element("Workflow Session") != nil
      }
      let tab = try XCTUnwrap(PaneOpenScreen.elements(host, named: "Workflow Session").first)
      destinations.append(try PaneOpenScreen.x(tab) > host.window.frame.midX)
      host.close()
      await model.stop()
    }
    XCTAssertEqual(destinations.count, 2)
    XCTAssertEqual(destinations.first, destinations.last)
  }

  @MainActor func test_B007_Root配信待ちのWorkflowメニューは無効() async throws {
    // Given
    let (_, model) = try await setup()
    let pane = try XCTUnwrap(model.panes.controller.allPaneIds.last)
    let host = await ViewHarness(
      HStack {
        PaneOpenMenu(model: model, pane: pane)
        RootListView(model: model, path: "/repo/a")
      })
    defer { host.close() }
    try await menu(host, button: "開く", selection: ["Workflow", "acceptance-flow"])
    try await eventuallyView(host) { host.texts().contains("Rootの配信を待っています") }
    // When / Then
    try await menu(host, button: "開く") { menu in
      XCTAssertFalse(try XCTUnwrap(menu.items.first { $0.title == "Workflow" }).isEnabled)
    }
    await model.stop()
  }

  @MainActor private func setup() async throws -> (TestHTTPClient, AppModel) {
    let http = TestHTTPClient()
    http.state.withLock { state in
      state.reviewThreads["/repo/a"] = .with { $0.items = [testReviewThread("retained-thread")] }
      for (id, title) in [
        ("agent-session", "Accepted Session"), ("workflow-session", "Workflow Session"),
      ] {
        state.sessionTabs[id] = .with {
          $0.session.id = id
          $0.session.workspaceWorktreePath = "/repo/a"
          $0.session.availability = .open
          $0.node.id = id == "agent-session" ? "session-tree" : "workflow-execution"
          $0.node.title = title
          $0.node.content.session.sessionID = id
        }
      }
      state.workflows.items = ["acceptance-flow", "other-flow"].map { name in
        .with { $0.name = name }
      }
    }
    let model = try await testModel(http)
    model.providers = [.with { $0.value = .codex }, .with { $0.value = .claude }]
    model.selectWorktree("/repo/a")
    try await eventually { model.layoutLoaded && model.workflows.count == 2 }
    try model.panes.restore(
      .split(
        id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: "first", tabs: [], active: nil),
        second: .pane(id: "second", tabs: [], active: nil)))
    model.panes.controller.focusPane(try XCTUnwrap(model.panes.controller.allPaneIds.first))
    return (http, model)
  }

  private func deliverRoot(_ http: TestHTTPClient) throws {
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [
      .with {
        $0.path = "/repo"
        $0.worktrees.items = [
          .with {
            $0.path = "/repo/a"
            $0.snapshot.nodes.items = [
              .with {
                $0.node.id = "workflow-execution"
                $0.node.title = "acceptance-flow"
                $0.node.contentKind.value = .session
                $0.node.sessionID = "workflow-session"
              }
            ]
          }
        ]
      }
    ]
    try http.publish(payload, target: "workspaces")
  }

  @MainActor private func menu(
    _ host: ViewHarness, button: String, selection: [String] = [], rightmost: Bool = false,
    inspect: @escaping @MainActor (NSMenu) throws -> Void = { _ in }
  ) async throws {
    try await PaneOpenMenuHarness().open(
      host, button: button, selection: selection, rightmost: rightmost, inspect: inspect)
  }
}

@MainActor private final class PaneOpenMenuHarness: NSObject {
  private var trackingMenu: NSMenu?
  private var chosen: (NSMenu, Int)?
  private var inspected = false
  private var tracking = false
  private var failure: Error?

  func open(
    _ host: ViewHarness, button: String, selection: [String] = [], rightmost: Bool = false,
    inspect: @escaping @MainActor (NSMenu) throws -> Void = { _ in }
  ) async throws {
    trackingMenu = nil
    NotificationCenter.default.addObserver(
      self, selector: #selector(menuBegan(_:)),
      name: NSMenu.didBeginTrackingNotification, object: nil)
    NotificationCenter.default.addObserver(
      self, selector: #selector(menuEnded(_:)),
      name: NSMenu.didEndTrackingNotification, object: nil)
    chosen = nil
    inspected = false
    tracking = false
    failure = nil
    let timer = Timer(timeInterval: 0.05, repeats: true) { [self] timer in
      let handled = MainActor.assumeIsolated {
        guard let menu = trackingMenu else { return false }
        defer {
          menu.cancelTracking()
          inspected = true
        }
        do {
          try inspect(menu)
          var current = menu
          for (offset, title) in selection.enumerated() {
            let index = try XCTUnwrap(current.items.firstIndex { $0.title == title })
            if offset == selection.count - 1 {
              XCTAssertTrue(current.items[index].isEnabled)
              chosen = (current, index)
            } else {
              current = try XCTUnwrap(current.items[index].submenu)
              current.update()
            }
          }
        } catch { failure = error }
        return true
      }
      if handled { timer.invalidate() }
    }
    RunLoop.main.add(timer, forMode: .common)
    RunLoop.main.add(timer, forMode: .eventTracking)
    defer {
      timer.invalidate()
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didBeginTrackingNotification, object: nil)
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didEndTrackingNotification, object: nil)
      trackingMenu = nil
    }
    if rightmost {
      try await eventuallyView(host) { PaneOpenScreen.elements(host, named: button).count == 2 }
      let buttons = try PaneOpenScreen.elements(host, named: button).sorted {
        try PaneOpenScreen.x($0) < PaneOpenScreen.x($1)
      }
      let target = try XCTUnwrap(buttons.last)
      await withCheckedContinuation { continuation in
        DispatchQueue.main.async {
          XCTAssertEqual(AXUIElementPerformAction(target, kAXPressAction as CFString), .success)
          continuation.resume()
        }
      }
    } else {
      try await host.press(button)
    }
    try await eventually { inspected && !tracking }
    if let failure { throw failure }
    if let (menu, index) = chosen { menu.performActionForItem(at: index) }
  }

  @MainActor @objc private func menuBegan(_ notification: Notification) {
    if trackingMenu == nil {
      trackingMenu = notification.object as? NSMenu
      tracking = true
    }
  }
  @objc private func menuEnded(_ notification: Notification) {
    if notification.object as? NSMenu === trackingMenu { tracking = false }
  }

}

@MainActor private enum PaneOpenScreen {
  static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
      return nil
    }
    return value
  }

  static func elements(_ host: ViewHarness, named name: String) -> [AXUIElement] {
    let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
    let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
    guard
      let window = windows.first(where: {
        attribute($0, kAXTitleAttribute) as? String == host.window.title
      })
    else { return [] }
    var visited: [AXUIElement] = []
    func collect(_ element: AXUIElement) -> [AXUIElement] {
      guard !visited.contains(where: { CFEqual($0, element) }) else { return [] }
      visited.append(element)
      let matches = [kAXIdentifierAttribute, kAXTitleAttribute, kAXDescriptionAttribute].contains {
        attribute(element, $0) as? String == name
      }
      return (matches ? [element] : [])
        + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(collect)
    }
    return collect(window)
  }

  static func x(_ element: AXUIElement) throws -> CGFloat {
    let value = try XCTUnwrap(attribute(element, kAXPositionAttribute))
    guard CFGetTypeID(value) == AXValueGetTypeID() else {
      throw CLIFailure(message: "Missing screen position")
    }
    var point = CGPoint.zero
    XCTAssertTrue(AXValueGetValue(value as! AXValue, .cgPoint, &point))
    return point.x
  }
}
