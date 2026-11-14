import AppKit
import SwiftProtobuf
import XCTest

@testable import Releash

final class SessionWorkflowMenuTests: XCTestCase {
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
      try await SessionMenuHarness().open(
        host, button: "開く", selection: ["Session", provider.displayName]
      ) { menu in
        let submenu = try XCTUnwrap(menu.items.first { $0.title == "Session" }?.submenu)
        submenu.update()
        XCTAssertEqual(submenu.items.map(\.title), model.providers.map(\.displayName))
      }
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
      try await SessionMenuHarness().open(host, button: "開く", selection: ["Terminal"]) { menu in
        XCTAssertEqual(
          menu.items.map(\.title), path == nil ? ["Terminal"] : ["Session", "Workflow", "Terminal"])
      }
      // Then
      try await eventually { model.panes.controller.tabs(inPane: second).count == 1 }
      XCTAssertTrue(model.panes.controller.tabs(inPane: first).isEmpty)
      let selected = try XCTUnwrap(model.panes.controller.selectedTab(inPane: second))
      XCTAssertEqual(model.panes.tab(selected.id)?.kind, .terminal)
    }
  }
}

@MainActor private final class SessionMenuHarness: NSObject {
  private var trackingMenu: NSMenu?
  private var chosen: (NSMenu, Int)?
  private var inspected = false
  private var tracking = false
  private var failure: Error?

  func open(
    _ host: ViewHarness, button: String, selection: [String] = [],
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
    try await host.press(button)
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
