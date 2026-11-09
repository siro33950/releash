import AppKit
import Carbon
import XCTest

@testable import Releash

final class StatusMenuTests: XCTestCase {
  @MainActor func test_メニューバー_設定項目は既存のSettingsウィンドウを開く() async throws {
    // Given
    let status = StatusMenu(model: AppModel()) {}
    defer { status.remove() }
    let menu = try XCTUnwrap(status.item.menu)
    let before = Set(NSApp.windows.map(ObjectIdentifier.init))
    defer {
      for window in NSApp.windows where !before.contains(ObjectIdentifier(window)) {
        window.close()
      }
    }
    status.menuNeedsUpdate(menu)
    // When
    menu.performActionForItem(at: try XCTUnwrap(menu.items.firstIndex { $0.title == "設定…" }))
    // Then
    try await eventually {
      NSApp.windows.contains {
        !before.contains(ObjectIdentifier($0)) && $0.isVisible
          && $0.identifier?.rawValue == "com_apple_SwiftUI_Settings_window"
      }
    }
  }

  @MainActor func test_メニューバー_画面が無くても残り項目から画面を復元して選択する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let loginItem = LoginItemRegistration()
    loginItem.status = { .notRegistered }
    let delegate = WorkspaceWindows(model: model, loginItem: loginItem)
    let event = NSAppleEventDescriptor(
      eventClass: kCoreEventClass, eventID: kAEOpenApplication,
      targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID),
      transactionID: AETransactionID(kAnyTransactionID))
    event.setParam(
      NSAppleEventDescriptor(enumCode: keyAELaunchedAsLogInItem), forKeyword: keyAEPropData)
    delegate.prepareLaunch(event: event)
    let status = try XCTUnwrap(delegate.statusMenu)
    defer {
      status.remove()
      for window in NSApp.windows where window.delegate === delegate { window.close() }
    }
    let menu = try XCTUnwrap(status.item.menu)
    try await eventually {
      http.state.withLock { $0.subscriptions.contains { $0.target == "execution-overview" } }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.executionOverview.status.value = .active
    payload.executionOverview.executions = [
      .with {
        $0.worktreePath = "/repo/linked"
        $0.worktreeName = "linked"
        $0.name = "build"
        $0.status.value = .active
      }
    ]
    try http.publish(payload, target: "execution-overview")
    try await eventually {
      status.menuNeedsUpdate(menu)
      return status.item.button?.toolTip == "Releash: active"
        && menu.items.first?.representedObject as? String == "/repo/linked"
    }
    // When
    for window in NSApp.windows where window.delegate === delegate { window.close() }
    // Then
    XCTAssertTrue(status.item.isVisible)
    XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApp))
    // When
    status.menuNeedsUpdate(menu)
    menu.performActionForItem(at: 0)
    // Then
    let window = try XCTUnwrap(NSApp.windows.first { $0.delegate === delegate && $0.isVisible })
    XCTAssertEqual(model.selectedWorktree, "/repo/linked")
    try await eventually { model.layoutLoaded }
    // When
    window.miniaturize(nil)
    menu.performActionForItem(at: 0)
    // Then
    XCTAssertFalse(window.isMiniaturized)
    XCTAssertTrue(window.isVisible)
    XCTAssertEqual(NSApp.windows.filter { $0.delegate === delegate && $0.isVisible }.count, 1)
    // When
    window.close()
    let open = try XCTUnwrap(menu.items.firstIndex { $0.title == "開く" })
    menu.performActionForItem(at: open)
    // Then
    XCTAssertTrue(NSApp.windows.contains { $0.delegate === delegate && $0.isVisible })
    XCTAssertTrue(status.item.isVisible)
    await model.stop()
  }
}
