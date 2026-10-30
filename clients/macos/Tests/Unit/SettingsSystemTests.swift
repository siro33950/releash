import Connect
import ServiceManagement
import SwiftProtobuf
import XCTest

@testable import Releash

final class SettingsSystemTests: XCTestCase {
  @MainActor func test_System購読_保存値とCLIの状態を受信して終了時に解放する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.desktopSettings.autoLaunch = true
      $0.desktopSettings.startMinimized = true
      $0.desktopSettings.performanceTelemetry = true
      $0.desktopSettings.crashReporting = true
      $0.daemonInfo.cliInstallation.status = .development
      $0.daemonInfo.cliInstallation.reason = "開発ビルドです"
    }
    let model = try await testModel(http)
    let state = SystemSettingsState()
    // When
    state.subscribe(model)
    state.subscribe(model)
    // Then
    try await eventually {
      state.settings?.autoLaunch == true && state.cliInstallation?.status == .development
    }
    XCTAssertEqual(state.settings?.startMinimized, true)
    XCTAssertEqual(state.settings?.performanceTelemetry, true)
    XCTAssertEqual(state.settings?.crashReporting, true)
    XCTAssertEqual(state.cliInstallation?.reason, "開発ビルドです")
    let targets = ["desktop-settings", "daemon-info"]
    let subscriptions = http.state.withLock {
      $0.subscriptions.filter { targets.contains($0.target) }
    }
    XCTAssertEqual(subscriptions.count, targets.count)
    XCTAssertTrue(subscriptions.allSatisfy { $0.args.isEmpty })
    // When
    state.unsubscribe(model)
    // Then
    try await eventually {
      let stopped = http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      }
      return subscriptions.filter { $0.target == "desktop-settings" }
        .allSatisfy { stopped.contains($0.subscriptionID) }
    }
    var payload = Releash_Client_V1_StatePayload()
    payload.daemonInfo.cliInstallation.status = .allowed
    try http.publish(payload, target: "daemon-info")
    try await eventually { model.daemonInfo?.cliInstallation.status == .allowed }
    XCTAssertEqual(state.cliInstallation?.status, .development)
    await model.stop()
  }

  @MainActor func test_System保存_各RPCに確定値と認証を渡して購読の値を表示し直す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = SystemSettingsState()
    state.subscribe(model)
    try await eventually { state.settings != nil }
    for value in [true, false] {
      // When
      await state.updateAutoLaunch(value, model: model)
      await state.updateStartMinimized(value, model: model)
      await state.updatePerformanceTelemetry(value, model: model)
      await state.updateCrashReporting(value, model: model)
      // Then
      let login = try Releash_Client_V1_UpdateLoginItemPreferenceRequest(
        serializedBytes: XCTUnwrap(http.requests("UpdateLoginItemPreference").last?.message))
      XCTAssertTrue(login.hasRequested)
      XCTAssertEqual(login.requested, value)
      let app = try Releash_Client_V1_UpdateAppSettingsRequest(
        serializedBytes: XCTUnwrap(http.requests("UpdateAppSettings").last?.message))
      XCTAssertTrue(app.hasApp)
      XCTAssertTrue(app.app.hasStartMinimized)
      XCTAssertEqual(app.app.startMinimized, value)
      XCTAssertEqual(try app.app.jsonString(), "{\"start_minimized\":\(value)}")
      let performance = try Releash_Client_V1_UpdatePerformanceTelemetryRequest(
        serializedBytes: XCTUnwrap(http.requests("UpdatePerformanceTelemetry").last?.message))
      XCTAssertTrue(performance.hasEnabled)
      XCTAssertEqual(performance.enabled, value)
      let crash = try Releash_Client_V1_UpdateCrashReportingRequest(
        serializedBytes: XCTUnwrap(http.requests("UpdateCrashReporting").last?.message))
      XCTAssertTrue(crash.hasEnabled)
      XCTAssertEqual(crash.enabled, value)
      XCTAssertEqual(state.settings?.autoLaunch, false)
      XCTAssertEqual(state.settings?.startMinimized, false)
      XCTAssertEqual(state.settings?.performanceTelemetry, false)
      XCTAssertEqual(state.settings?.crashReporting, false)
    }
    for method in [
      "UpdateLoginItemPreference", "UpdateAppSettings", "UpdatePerformanceTelemetry",
      "UpdateCrashReporting",
    ] {
      XCTAssertEqual(http.requests(method).count, 2)
      XCTAssertEqual(http.requests(method).last?.headers["authorization"], ["Bearer operator-test"])
    }
    XCTAssertNil(state.failure)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.desktopSettings = .with {
      $0.autoLaunch = true
      $0.startMinimized = true
      $0.performanceTelemetry = true
      $0.crashReporting = true
    }
    try http.publish(payload, target: "desktop-settings")
    // Then
    try await eventually { state.settings == payload.desktopSettings }
    // When
    state.unsubscribe(model)
    let reopened = SystemSettingsState()
    reopened.subscribe(model)
    // Then
    try await eventually { reopened.settings == payload.desktopSettings }
    reopened.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_System保存_失敗の理由を表示して保存済みの値を保つ() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = SystemSettingsState()
    state.subscribe(model)
    try await eventually { state.settings != nil }
    let saved = state.settings
    for method in [
      "UpdateLoginItemPreference", "UpdateAppSettings", "UpdatePerformanceTelemetry",
      "UpdateCrashReporting", "InstallCli", "CheckLoginRegistration",
    ] {
      http.state.withLock { $0.failedMethod = method }
      // When
      switch method {
      case "UpdateLoginItemPreference": await state.updateAutoLaunch(true, model: model)
      case "UpdateAppSettings": await state.updateStartMinimized(true, model: model)
      case "UpdatePerformanceTelemetry": await state.updatePerformanceTelemetry(true, model: model)
      case "UpdateCrashReporting": await state.updateCrashReporting(true, model: model)
      case "InstallCli": await state.installCli(model: model)
      default: await state.checkLoginRegistration("/Applications/Releash.app", model: model)
      }
      // Then
      XCTAssertEqual(state.failure, "\(method) failed")
      XCTAssertEqual(state.settings, saved)
      XCTAssertNil(state.cliResult)
      XCTAssertNil(state.loginRegistration)
      XCTAssertTrue(state.saving.isEmpty)
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    await state.updateCrashReporting(true, model: model)
    // Then
    XCTAssertNil(state.failure)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_ログイン登録可否_アプリの実行パスを渡しサーバの判定と理由を保持する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = SystemSettingsState()
    let path = "/Applications/Releash Preview.app/Contents/MacOS/Releash Preview"
    for status: Releash_Client_V1_LoginRegistrationStatus in [.allowed, .translocated, .readOnly] {
      http.state.withLock {
        $0.loginRegistration.status = status
        $0.loginRegistration.reason = "server reason: \(status)"
      }
      // When
      await state.checkLoginRegistration(path, model: model)
      // Then
      let request = try Releash_Client_V1_CheckLoginRegistrationRequest(
        serializedBytes: XCTUnwrap(http.requests("CheckLoginRegistration").last?.message))
      XCTAssertEqual(request.executablePath, path)
      XCTAssertEqual(state.loginRegistration?.status, status)
      XCTAssertEqual(state.loginRegistration?.reason, "server reason: \(status)")
      XCTAssertEqual(
        http.requests("CheckLoginRegistration").last?.headers["authorization"],
        ["Bearer operator-test"])
    }
    // When
    await state.checkLoginRegistration(nil, model: model)
    // Then
    XCTAssertEqual(state.failure, "アプリの実行ファイルのパスを取得できません")
    XCTAssertEqual(http.requests("CheckLoginRegistration").count, 3)
    await model.stop()
  }

  @MainActor func test_ログイン項目_差し替えた承認状態を表示へ反映する() {
    // Given
    let state = SystemSettingsState()
    for status: SMAppService.Status in [.notRegistered, .enabled, .requiresApproval, .notFound] {
      state.loginItemStatus = { status }
      // When
      state.refreshLoginItemStatus()
      // Then
      XCTAssertEqual(state.requiresApproval, status == .requiresApproval)
    }
  }

  @MainActor func test_CLI設置_設置した結果と設置済みの結果をパスとともに受け取る() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = SystemSettingsState()
    for status: Releash_Client_V1_CliInstallStatus in [.installed, .alreadyInstalled] {
      http.state.withLock {
        $0.cliInstallResult.status = status
        $0.cliInstallResult.path = "/usr/local/bin/releash"
      }
      // When
      await state.installCli(model: model)
      // Then
      XCTAssertEqual(state.cliResult?.status, status)
      XCTAssertEqual(state.cliResult?.path, "/usr/local/bin/releash")
      XCTAssertNil(state.failure)
      XCTAssertTrue(state.saving.isEmpty)
      XCTAssertEqual(
        http.requests("InstallCli").last?.headers["authorization"], ["Bearer operator-test"])
    }
    await model.stop()
  }

  @MainActor func test_System未接続_変更を送らず理由を表示する() async {
    // Given
    let state = SystemSettingsState()
    // When
    await state.updateAutoLaunch(true, model: AppModel())
    // Then
    XCTAssertEqual(state.failure, "サーバに接続されていません")
    XCTAssertNil(state.settings)
    XCTAssertTrue(state.saving.isEmpty)
  }
}
