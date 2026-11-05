import AppKit
import Carbon
import Connect
import ServiceManagement
import XCTest

@testable import Releash

final class WorkspaceWindowsTests: XCTestCase {
  @MainActor func test_起動_ログイン起動と初回設定に応じてウィンドウを表示する() async throws {
    for login in [false, true] {
      for autoLaunch in [false, true] {
        for minimized in [false, true] {
          // Given
          let http = TestHTTPClient()
          http.state.withLock {
            $0.desktopSettings.autoLaunch = autoLaunch
            $0.desktopSettings.startMinimized = minimized
            $0.loginRegistration.status = .readOnly
          }
          let model = try await testModel(http)
          let item = LoginItemRegistration()
          var unregistered = 0
          item.status = { .enabled }
          item.unregister = { unregistered += 1 }
          item.register = { XCTFail("登録できない場所で登録しました") }
          let delegate = WorkspaceWindows(model: model, loginItem: item)
          // When
          delegate.prepareLaunch(event: login ? loginEvent() : nil)
          try await eventually {
            autoLaunch ? http.requests("CheckLoginRegistration").count == 1 : unregistered == 1
          }
          // Then
          let hidden = login && autoLaunch && minimized
          try await eventually { self.windows(delegate).count == (hidden ? 0 : 1) }
          XCTAssertTrue(windows(delegate).allSatisfy(\.isVisible))
          // When
          var payload = Releash_Client_V1_StatePayload()
          payload.desktopSettings.autoLaunch = !autoLaunch
          payload.desktopSettings.startMinimized = !minimized
          try http.publish(payload, target: "desktop-settings")
          try await eventually {
            autoLaunch ? unregistered == 1 : http.requests("CheckLoginRegistration").count == 1
          }
          // Then
          XCTAssertEqual(windows(delegate).count, hidden ? 0 : 1)
          XCTAssertTrue(http.requests("InstallCli").isEmpty)
          for window in windows(delegate) { window.close() }
          await model.stop()
        }
      }
    }
  }

  @MainActor func test_ログイン起動_接続失敗で画面を表示し明示の再表示後は画面を増やさない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = AppModel(
      cli: CLI(
        executable: URL(fileURLWithPath: "/test/releash"),
        run: { _, _ in throw CLIFailure(message: "connection failed") }),
      httpClient: http)
    let item = LoginItemRegistration()
    item.status = { .notRegistered }
    let delegate = WorkspaceWindows(model: model, loginItem: item)
    defer { for window in windows(delegate) { window.close() } }
    // When
    delegate.prepareLaunch(event: loginEvent())
    await delegate.start()
    // Then
    XCTAssertNotNil(model.connectionFailure)
    XCTAssertEqual(windows(delegate).count, 1)
    let original = try XCTUnwrap(windows(delegate).first)
    XCTAssertTrue(original.isVisible)
    XCTAssertTrue(http.requests("InstallCli").isEmpty)
    // When
    XCTAssertFalse(delegate.applicationShouldHandleReopen(NSApp, hasVisibleWindows: false))
    // Then
    XCTAssertEqual(windows(delegate).count, 1)
    XCTAssertTrue(windows(delegate).first === original)
    XCTAssertTrue(windows(delegate)[0].isVisible)
    XCTAssertTrue(http.requests("OpenStateStream").isEmpty)
    await model.stop()
  }

  @MainActor func test_再起動要求_既存画面を復元し最後の画面を閉じても終了しない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    item.status = { .notRegistered }
    let delegate = WorkspaceWindows(model: model, loginItem: item)
    delegate.prepareLaunch(event: nil)
    defer { for window in windows(delegate) { window.close() } }
    let original = try XCTUnwrap(windows(delegate).first)
    let streams = http.requests("OpenStateStream").count
    // When
    original.miniaturize(nil)
    XCTAssertFalse(delegate.applicationShouldHandleReopen(NSApp, hasVisibleWindows: false))
    // Then
    XCTAssertEqual(windows(delegate).count, 1)
    XCTAssertTrue(windows(delegate).first === original)
    XCTAssertFalse(original.isMiniaturized)
    XCTAssertTrue(original.isVisible)
    // When
    original.close()
    // Then
    XCTAssertFalse(original.isVisible)
    XCTAssertFalse(original.isMiniaturized)
    XCTAssertTrue(windows(delegate).isEmpty)
    XCTAssertFalse(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApp))
    XCTAssertTrue(model.connected)
    // When
    delegate.showWindow()
    // Then
    XCTAssertEqual(windows(delegate).count, 1)
    XCTAssertFalse(windows(delegate).first === original)
    XCTAssertEqual(http.requests("OpenStateStream").count, streams)
    XCTAssertEqual(delegate.applicationShouldTerminate(NSApp), .terminateNow)
    XCTAssertTrue(model.connected)
    XCTAssertTrue(http.requests("AbortWorkflow").isEmpty)
    XCTAssertTrue(http.requests("InstallCli").isEmpty)
    await model.stop()
  }

  @MainActor func test_ログイン起動_設定受信前に明示で開いた画面を重複して開かない() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    item.status = { .enabled }
    var synchronized = false
    item.unregister = { synchronized = true }
    let delegate = WorkspaceWindows(model: model, loginItem: item)
    defer { for window in windows(delegate) { window.close() } }
    delegate.prepareLaunch(event: loginEvent())
    XCTAssertTrue(windows(delegate).isEmpty)
    // When
    delegate.showWindow()
    let original = try XCTUnwrap(windows(delegate).first)
    try await eventually { synchronized }
    // Then
    XCTAssertEqual(windows(delegate).count, 1)
    XCTAssertTrue(windows(delegate).first === original)
    await model.stop()
  }

  @MainActor func test_ログイン項目_購読で登録解除し失敗は画面のエラーへ渡す() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.desktopSettings.autoLaunch = true
      $0.desktopSettings.startMinimized = true
      $0.loginRegistration.status = .allowed
    }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    var status: SMAppService.Status = .notRegistered
    var registered = 0
    var unregistered = 0
    item.status = { status }
    item.register = {
      registered += 1
      status = .enabled
    }
    item.unregister = {
      unregistered += 1
      status = .notRegistered
    }
    let delegate = WorkspaceWindows(model: model, loginItem: item)
    delegate.prepareLaunch(event: loginEvent())
    // Then
    try await eventually { registered == 1 }
    XCTAssertTrue(windows(delegate).isEmpty)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.desktopSettings.autoLaunch = false
    try http.publish(payload, target: "desktop-settings")
    // Then
    try await eventually { unregistered == 1 }
    // When
    item.register = { throw CLIFailure(message: "registration failed") }
    payload.desktopSettings.autoLaunch = true
    try http.publish(payload, target: "desktop-settings")
    // Then
    try await eventually { model.failure == "registration failed" }
    XCTAssertTrue(windows(delegate).isEmpty)
    XCTAssertTrue(http.requests("InstallCli").isEmpty)
    await model.stop()
  }

  @MainActor func test_終了_登録完了と失敗を待ち期限超過でも一度だけ終了を許可する() async throws {
    for outcome in ["success", "failure", "timeout"] {
      // Given
      let http = TestHTTPClient()
      http.state.withLock {
        $0.desktopSettings.autoLaunch = true
        $0.desktopSettings.startMinimized = true
        $0.deferredTerminalMethod = "CheckLoginRegistration"
      }
      let model = try await testModel(http)
      let item = LoginItemRegistration()
      item.status = { .notRegistered }
      var registered = 0
      item.register = { registered += 1 }
      let delegate = WorkspaceWindows(model: model, loginItem: item)
      if outcome == "timeout" { delegate.terminationTimeout = .milliseconds(20) }
      var replies: [Bool] = []
      delegate.replyToTermination = { replies.append($0) }
      delegate.prepareLaunch(event: loginEvent())
      try await eventually { http.state.withLock { $0.pendingTerminal != nil } }
      let completion = try XCTUnwrap(http.state.withLock { $0.pendingTerminal })
      // When
      XCTAssertEqual(delegate.applicationShouldTerminate(NSApp), .terminateLater)
      XCTAssertEqual(delegate.applicationShouldTerminate(NSApp), .terminateLater)
      XCTAssertTrue(replies.isEmpty)
      if outcome == "timeout" { try await eventually { replies == [true] } }
      let allowed = Releash_Client_V1_LoginRegistrationResult.with { $0.status = .allowed }
      completion(
        HTTPResponse(
          code: outcome == "failure" ? .internalError : .ok,
          headers: ["content-type": ["application/proto"]],
          message: try allowed.serializedData(), trailers: [:],
          error: outcome == "failure"
            ? ConnectError(code: .internalError, message: "registration failed") : nil,
          tracingInfo: nil))
      // Then
      try await eventually { replies == [true] }
      await Task.yield()
      XCTAssertEqual(registered, outcome == "success" ? 1 : 0)
      XCTAssertEqual(replies, [true])
      await model.stop()
    }
  }

  @MainActor private func windows(_ delegate: WorkspaceWindows) -> [NSWindow] {
    NSApp.windows.filter { $0.delegate === delegate && ($0.isVisible || $0.isMiniaturized) }
  }

  private func loginEvent() -> NSAppleEventDescriptor {
    let event = NSAppleEventDescriptor(
      eventClass: kCoreEventClass, eventID: kAEOpenApplication,
      targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID),
      transactionID: AETransactionID(kAnyTransactionID))
    event.setParam(
      NSAppleEventDescriptor(enumCode: keyAELaunchedAsLogInItem), forKeyword: keyAEPropData)
    return event
  }
}
