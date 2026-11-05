import AppKit
import Connect
import XCTest

@testable import Releash

final class StatusMenuTests: XCTestCase {
  @MainActor func test_メニューバー_購読した三状態を表示し未接続時は状態と一覧を消す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var opened = 0
    let status = StatusMenu(model: model) { opened += 1 }
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "execution-overview" } }
    }
    // When
    for value: Releash_Client_V1_WorkspaceStatusClassification.Value in [
      .attention, .active, .idle,
    ] {
      var payload = Releash_Client_V1_StatePayload()
      payload.executionOverview.status.value = value
      payload.executionOverview.executions = [
        .with {
          $0.worktreePath = "/repo/b"
          $0.worktreeName = "feature-b"
          $0.name = "review"
          $0.status.value = .active
        },
        .with {
          $0.worktreePath = "/repo/a"
          $0.worktreeName = "feature-a"
          $0.name = "Claude Code"
          $0.status.value = .attention
        },
      ]
      try http.publish(payload, target: "execution-overview")
      // Then
      try await eventually { status.item.button?.toolTip == "Releash: \(value)" }
      XCTAssertEqual(status.item.button?.contentTintColor, NSColor(UIStyle.statusColor("\(value)")))
      XCTAssertEqual(status.item.button?.image?.isTemplate, true)
    }
    status.menuNeedsUpdate(menu)
    XCTAssertEqual(
      Array(menu.items.prefix(2)).map(\.title),
      [
        "feature-a — Claude Code (attention)", "feature-b — review (active)",
      ])
    XCTAssertTrue(menu.items.prefix(2).allSatisfy { $0.image != nil })
    // When
    menu.performActionForItem(at: 0)
    // Then
    XCTAssertEqual(opened, 1)
    XCTAssertEqual(model.selectedWorktree, "/repo/a")
    try await eventually {
      http.state.withLock {
        $0.subscriptions.contains {
          $0.target == "workspace-state" && $0.args == ["/repo/a", "/repo/a"]
        }
      }
    }
    // When
    model.reconnecting = true
    // Then
    try await eventually { status.item.button?.contentTintColor == nil }
    status.menuNeedsUpdate(menu)
    XCTAssertEqual(menu.items.first?.title, "開く")
    XCTAssertTrue(menu.items.allSatisfy { $0.representedObject == nil })
    // When
    model.reconnecting = false
    // Then
    try await Task.sleep(for: .milliseconds(20))
    XCTAssertNil(status.item.button?.contentTintColor)
    var payload = Releash_Client_V1_StatePayload()
    payload.executionOverview.status.value = .idle
    try http.publish(payload, target: "execution-overview")
    try await eventually { status.item.button?.contentTintColor == NSColor(UIStyle.idle) }
    status.menuNeedsUpdate(menu)
    XCTAssertEqual(menu.items.first?.title, "開く")
    // When
    let previous = http.state.withLock {
      $0.subscriptions.last { $0.target == "execution-overview" }?.subscriptionID
    }
    let stream = try XCTUnwrap(http.state.withLock { $0.streams.last })
    stream.receiveClose(
      .unavailable, [:], ConnectError(code: .unavailable, message: "disconnected"))
    // Then
    try await eventually { model.reconnecting && status.item.button?.contentTintColor == nil }
    try await eventually {
      !model.reconnecting && status.item.button?.toolTip == "Releash: idle"
        && http.state.withLock {
          $0.subscriptions.last { $0.target == "execution-overview" }?.subscriptionID != previous
        }
    }
    // When
    model.connected = false
    // Then
    try await eventually { status.item.button?.contentTintColor == nil }
    await model.stop()
  }

  @MainActor func test_実行概要_正常木と個別の失敗を表示し回復後に失敗表示を消す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let status = StatusMenu(model: model) {}
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "execution-overview" } }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.executionOverview.status.value = .attention
    payload.executionOverview.executions = [
      .with {
        $0.worktreePath = "/repo"
        $0.worktreeName = "repo"
        $0.name = "healthy"
        $0.status.value = .idle
      }
    ]
    payload.executionOverview.failures = [
      "broken": .with {
        $0.code = 13
        $0.message = "read failed"
      }
    ]
    // When / Then
    for onlyFailure in [false, true] {
      if onlyFailure { payload.executionOverview.executions = [] }
      try http.publish(payload, target: "execution-overview")
      try await eventually {
        status.menuNeedsUpdate(menu)
        return menu.items.contains { $0.title == "実行木 broken を読み取れません: read failed" }
          && (menu.items.contains { $0.representedObject as? String == "/repo" }) == !onlyFailure
      }
      XCTAssertEqual(status.item.button?.toolTip, "Releash: attention")
      XCTAssertFalse(try XCTUnwrap(menu.items.first { $0.title.contains("broken") }).isEnabled)
      XCTAssertTrue(menu.items.contains { $0.title == "開く" })
    }
    // When
    payload.executionOverview.failures = [:]
    payload.executionOverview.status.value = .idle
    try http.publish(payload, target: "execution-overview")
    // Then
    try await eventually {
      status.menuNeedsUpdate(menu)
      return status.item.button?.toolTip == "Releash: idle"
        && !menu.items.contains { $0.title.contains("broken") }
    }
    await model.stop()
  }

  @MainActor func test_メニューバー_標準操作へ渡す() throws {
    // Given
    let model = AppModel()
    var opened = 0
    let status = StatusMenu(model: model) { opened += 1 }
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    status.menuNeedsUpdate(menu)
    // When
    menu.performActionForItem(at: 0)
    // Then
    XCTAssertEqual(opened, 1)
    let about = try XCTUnwrap(menu.items.first { $0.title == "Releash について" })
    XCTAssertTrue(about.target === NSApp)
    XCTAssertEqual(about.action, #selector(NSApplication.orderFrontStandardAboutPanel(_:)))
    let quit = try XCTUnwrap(menu.items.first { $0.title == "Releash を終了" })
    XCTAssertTrue(quit.target === NSApp)
    XCTAssertEqual(quit.action, #selector(NSApplication.terminate(_:)))
    // Given
    let original = NSApp.mainMenu
    defer { NSApp.mainMenu = original }
    let main = NSMenu()
    let app = NSMenuItem()
    main.addItem(app)
    app.submenu = NSMenu()
    let receiver = SettingsReceiver()
    let settings = NSMenuItem(
      title: "設定…", action: #selector(SettingsReceiver.open(_:)), keyEquivalent: ",")
    settings.target = receiver
    app.submenu?.addItem(settings)
    NSApp.mainMenu = main
    // When
    menu.performActionForItem(at: 1)
    // Then
    XCTAssertEqual(receiver.opened, 1)
  }
}

@MainActor private final class SettingsReceiver: NSObject {
  var opened = 0
  @objc func open(_ sender: Any?) { opened += 1 }
}
