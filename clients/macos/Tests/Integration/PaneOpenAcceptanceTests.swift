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
    let temporary = try FileManager.default.temporaryDirectory.resourceValues(forKeys: [
      .canonicalPathKey
    ])
    let root = URL(fileURLWithPath: try XCTUnwrap(temporary.canonicalPath))
      .appendingPathComponent(UUID().uuidString)
    let data = root.appendingPathComponent("data")
    let repo = root.appendingPathComponent("repo")
    for directory in [data, repo] {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }
    let daemon = Process()
    defer {
      if daemon.isRunning {
        daemon.terminate()
        daemon.waitUntilExit()
      }
      try? FileManager.default.removeItem(at: root)
    }
    for arguments in [
      ["init", "-b", "main"],
      [
        "-c", "user.name=Acceptance", "-c", "user.email=test@example.invalid", "-c",
        "commit.gpgsign=false", "commit", "--allow-empty", "-m", "initial",
      ],
    ] {
      let git = Process()
      git.executableURL = URL(fileURLWithPath: "/usr/bin/git")
      git.arguments = arguments
      git.currentDirectoryURL = repo
      git.standardOutput = FileHandle.nullDevice
      git.standardError = FileHandle.nullDevice
      try git.run()
      git.waitUntilExit()
      XCTAssertEqual(git.terminationStatus, 0)
    }
    let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
    let provider = root.appendingPathComponent("claude")
    let cliPath = helpers.appendingPathComponent("releash").path.replacingOccurrences(
      of: "'", with: "'\"'\"'")
    try """
    #!/bin/sh
    if [ "$1" = --version ]; then echo 'claude 1.0.0'; exit; fi
    printf '{"session_id":"provider-%s","transcript_path":"provider://fixture/%s","hook_event_name":"SessionStart"}' "$RELEASH_SESSION_ID" "$RELEASH_SESSION_ID" | '\(cliPath)' hook receive --provider claude || exit 90
    exec /bin/cat

    """.write(to: provider, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: provider.path)
    daemon.executableURL = helpers.appendingPathComponent("releashd")
    daemon.arguments = ["--data-dir", data.path]
    daemon.currentDirectoryURL = repo
    var environment = ProcessInfo.processInfo.environment
    environment["RELEASH_DATA_DIR"] = data.path
    environment["RELEASH_WORKFLOWS_DIR"] = data.appendingPathComponent("workflows").path
    environment["SHELL"] = "/bin/sh"
    daemon.environment = environment
    daemon.standardInput = FileHandle.nullDevice
    daemon.standardOutput = FileHandle.nullDevice
    daemon.standardError = FileHandle.nullDevice
    try daemon.run()
    let discovery = data.appendingPathComponent("client-api.json")
    try await eventually { FileManager.default.fileExists(atPath: discovery.path) }
    let status = try JSONSerialization.data(withJSONObject: [
      "running": true, "compatibility": "compatible", "release_compatibility": "compatible",
      "discovery_file": discovery.path,
    ])
    let model = AppModel(
      cli: CLI(executable: helpers.appendingPathComponent("releash"), run: { _, _ in status }))
    await model.start()
    try await eventually { model.connected }
    let client = try XCTUnwrap(model.client)
    var configure = Releash_Client_V1_UpdateProviderExecutableRequest()
    configure.provider = "claude"
    configure.executable = provider.path
    _ = try await client.updateProviderExecutable(request: configure, headers: model.headers).result
      .get()
    var add = Releash_Client_V1_AddRepoPathRequest()
    add.path = repo.path
    _ = try await client.addRepoPath(request: add, headers: model.headers).result.get()
    var save = Releash_Client_V1_SaveWorkspaceStateRequest()
    save.worktreeName = repo.path
    save.state.version = 1
    save.state.etag = "0"
    save.state.paneLayout =
      PaneLayout.split(
        id: "split", axis: .horizontal, ratio: 0.5,
        first: .pane(id: "first", tabs: [], active: nil),
        second: .pane(
          id: "second", tabs: [PaneTab(id: "existing-terminal", kind: .terminal)],
          active: "existing-terminal")
      ).message
    _ = try await client.saveWorkspaceState(request: save, headers: model.headers).result.get()
    model.selectWorktree(repo.path)
    try await eventually { model.layoutLoaded && model.panes.controller.allPaneIds.count == 2 }
    model.panes.controller.focusPane(try XCTUnwrap(model.panes.controller.allPaneIds.first))
    let host = await ViewHarness(WorkspacePaneView(model: model))
    defer { host.close() }
    // When
    try await menu(host, button: "開く", selection: ["Session", "Claude"], rightmost: true)
    // Then
    func sessions() -> [Releash_Client_V1_WorkspaceNode] {
      model.repositories.flatMap(\.worktrees.items).filter { $0.path == repo.path }
        .flatMap(\.snapshot.nodes.items).compactMap {
          guard case .node(let node) = $0.variant, node.hasSessionID else { return nil }
          return node
        }
    }
    let deadline = ContinuousClock.now + .seconds(45)
    while sessions().isEmpty && model.failure == nil && ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(20))
    }
    let session = try XCTUnwrap(
      sessions().first, model.failure ?? "Session was not delivered to the selected worktree")
    XCTAssertEqual(session.provider.value, .claude)
    try await eventuallyView(host) {
      host.element("terminal:" + session.sessionID) != nil && host.element(session.title) != nil
    }
    let tab = try XCTUnwrap(
      PaneOpenScreen.elements(host, named: session.title).first {
        PaneOpenScreen.attribute($0, kAXRoleAttribute) as? String == kAXButtonRole
      })
    XCTAssertGreaterThan(try PaneOpenScreen.x(tab), host.window.frame.midX)
    XCTAssertEqual(PaneOpenScreen.attribute(tab, kAXSelectedAttribute) as? Bool, true)

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
      let index = try XCTUnwrap(menu.items.firstIndex { $0.title == "Workflow" })
      XCTAssertEqual(index, 1)
      XCTAssertFalse(menu.items[index].isEnabled)
      XCTAssertNil(menu.items[index].submenu)
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
