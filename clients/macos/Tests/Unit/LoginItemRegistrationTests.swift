import Carbon
import Connect
import ServiceManagement
import SwiftProtobuf
import XCTest

@testable import Releash

final class LoginItemRegistrationTests: XCTestCase {
  @MainActor func test_ログイン起動_起動イベントの属性だけで判定する() {
    // Given
    let event = NSAppleEventDescriptor(
      eventClass: kCoreEventClass, eventID: kAEOpenApplication,
      targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID),
      transactionID: AETransactionID(kAnyTransactionID))
    // Then
    XCTAssertFalse(WorkspaceWindows.isLoginItemLaunch(nil))
    XCTAssertFalse(WorkspaceWindows.isLoginItemLaunch(event))
    for value in [keyAELaunchedAsServiceItem, keyAELaunchedAsLogInItem] {
      // When
      event.setParam(NSAppleEventDescriptor(enumCode: value), forKeyword: keyAEPropData)
      // Then
      XCTAssertEqual(WorkspaceWindows.isLoginItemLaunch(event), value == keyAELaunchedAsLogInItem)
    }
    let reopen = NSAppleEventDescriptor(
      eventClass: kCoreEventClass, eventID: kAEReopenApplication,
      targetDescriptor: nil, returnID: AEReturnID(kAutoGenerateReturnID),
      transactionID: AETransactionID(kAnyTransactionID))
    reopen.setParam(
      NSAppleEventDescriptor(enumCode: keyAELaunchedAsLogInItem), forKeyword: keyAEPropData)
    XCTAssertFalse(WorkspaceWindows.isLoginItemLaunch(reopen))
  }

  @MainActor func test_ログイン項目_登録可否と現在の登録状態に合わせて同期する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    item.executablePath = "/Applications/Releash Preview.app/Contents/MacOS/Releash Preview"
    for allowed: Releash_Client_V1_LoginRegistrationStatus in [.allowed, .translocated, .readOnly] {
      http.state.withLock { $0.loginRegistration.status = allowed }
      for status: SMAppService.Status in [.notRegistered, .notFound, .enabled, .requiresApproval] {
        var registered = 0
        var unregistered = 0
        item.status = { status }
        item.register = { registered += 1 }
        item.unregister = { unregistered += 1 }
        // When
        try await item.synchronize(autoLaunch: true, model: model)
        // Then
        XCTAssertEqual(
          registered,
          allowed == .allowed && (status == .notRegistered || status == .notFound) ? 1 : 0)
        XCTAssertEqual(unregistered, 0)
        let request = try Releash_Client_V1_CheckLoginRegistrationRequest(
          serializedBytes: XCTUnwrap(http.requests("CheckLoginRegistration").last?.message))
        XCTAssertEqual(request.executablePath, item.executablePath)
        XCTAssertEqual(
          http.requests("CheckLoginRegistration").last?.headers["authorization"],
          ["Bearer operator-test"])
        // When
        try await item.synchronize(autoLaunch: false, model: model)
        // Then
        XCTAssertEqual(unregistered, status == .enabled || status == .requiresApproval ? 1 : 0)
      }
    }
    XCTAssertTrue(http.requests("InstallCli").isEmpty)
    await model.stop()
  }

  @MainActor func test_ログイン項目_改名後に新しいアプリを登録し直す() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.loginRegistration.status = .allowed }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    item.executablePath = "/Applications/Releash.app/Contents/MacOS/Releash"
    item.status = { .enabled }
    var operations: [String] = []
    item.unregister = { operations.append("unregister") }
    item.register = { operations.append("register") }
    // When
    try await item.synchronize(autoLaunch: true, model: model)
    try await item.synchronize(autoLaunch: true, model: model)
    // Then
    XCTAssertEqual(operations, ["unregister", "register"])
    await model.stop()
  }

  @MainActor func test_ログイン項目_登録後の再通知では重複登録せず失われた登録を復元する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.loginRegistration.status = .allowed }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    var status: SMAppService.Status = .notRegistered
    var calls = 0
    item.status = { status }
    item.register = {
      calls += 1
      status = .enabled
    }
    // When
    try await item.synchronize(autoLaunch: true, model: model)
    try await item.synchronize(autoLaunch: true, model: model)
    // Then
    XCTAssertEqual(calls, 1)
    // When
    status = .notRegistered
    try await item.synchronize(autoLaunch: true, model: model)
    // Then
    XCTAssertEqual(calls, 2)
    await model.stop()
  }

  @MainActor func test_ログイン項目_判定とOSの失敗を呼び出し元へ返す() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.loginRegistration.status = .allowed }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    var status: SMAppService.Status = .notRegistered
    item.status = { status }
    item.register = { throw CLIFailure(message: "register failed") }
    item.unregister = { throw CLIFailure(message: "unregister failed") }
    for failure in ["CheckLoginRegistration", "register", "unregister", "path", "connection"] {
      http.state.withLock { $0.failedMethod = failure == "CheckLoginRegistration" ? failure : nil }
      item.executablePath =
        failure == "path" ? nil : "/Applications/Releash.app/Contents/MacOS/Releash"
      status = failure == "unregister" ? .enabled : .notRegistered
      // When
      do {
        try await item.synchronize(
          autoLaunch: failure != "unregister", model: failure == "connection" ? AppModel() : model)
        XCTFail("同期が失敗を返しませんでした: \(failure)")
      } catch {
        // Then
        let expected =
          [
            "path": "アプリの実行ファイルのパスを取得できません",
            "connection": "サーバに接続されていません",
          ][failure] ?? "\(failure) failed"
        XCTAssertEqual(model.failureMessage(error), expected)
      }
    }
    await model.stop()
  }

  @MainActor func test_ログイン項目_古い設定の登録可否が後から返っても登録しない() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.deferredTerminalMethod = "CheckLoginRegistration" }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    item.status = { .notRegistered }
    var calls = 0
    item.register = { calls += 1 }
    let task = Task { try await item.synchronize(autoLaunch: true, model: model) }
    try await eventually { http.state.withLock { $0.pendingTerminal != nil } }
    // When
    task.cancel()
    try await item.synchronize(autoLaunch: false, model: model)
    let completion = try XCTUnwrap(http.state.withLock { $0.pendingTerminal })
    let allowed = Releash_Client_V1_LoginRegistrationResult.with { $0.status = .allowed }
    completion(
      HTTPResponse(
        code: .ok, headers: ["content-type": ["application/proto"]],
        message: try allowed.serializedData(), trailers: [:],
        error: nil, tracingInfo: nil))
    // Then
    do {
      try await task.value
      XCTFail("古い設定の同期が取り消されませんでした")
    } catch is CancellationError {} catch { XCTFail("\(error)") }
    XCTAssertEqual(calls, 0)
    await model.stop()
  }
}
