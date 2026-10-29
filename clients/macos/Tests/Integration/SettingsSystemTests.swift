import ServiceManagement
import SwiftUI
import XCTest

@testable import Releash

final class SettingsSystemTests: XCTestCase {
  @MainActor func test_System未接続_未受信の設定を変更できない() async throws {
    // Given
    let host = await ViewHarness(SystemSettingsView(model: AppModel()))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("settings-auto-launch") != nil }
    for id in [
      "settings-auto-launch", "settings-start-minimized", "settings-performance-telemetry",
      "settings-crash-reporting", "settings-install-cli",
    ] {
      XCTAssertEqual(host.element(id)?.enabled, false, id)
    }
  }

  @MainActor func test_System表示_購読値と起動条件と登録できない理由を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.desktopSettings.startMinimized = true
      $0.desktopSettings.performanceTelemetry = true
      $0.daemonInfo.cliInstallation.status = .readOnly
      $0.daemonInfo.cliInstallation.reason = "CLIの設置先は読み取り専用です"
      $0.loginRegistration.status = .translocated
      $0.loginRegistration.reason = "アプリをApplicationsに移動してください"
    }
    let model = try await testModel(http)
    let host = await ViewHarness(SystemSettingsView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) {
      host.element("settings-start-minimized")?.value == "1"
        && host.texts().contains("アプリをApplicationsに移動してください")
        && host.texts().contains("CLIの設置先は読み取り専用です")
    }
    XCTAssertEqual(host.element("settings-auto-launch")?.value, "0")
    XCTAssertEqual(host.element("settings-start-minimized")?.enabled, false)
    XCTAssertEqual(host.element("settings-performance-telemetry")?.value, "1")
    XCTAssertEqual(host.element("settings-crash-reporting")?.value, "0")
    XCTAssertEqual(host.element("settings-install-cli")?.enabled, false)
    XCTAssertTrue(host.texts().contains("/usr/local/bin/releash"))
    let request = try Releash_Client_V1_CheckLoginRegistrationRequest(
      serializedBytes: XCTUnwrap(http.requests("CheckLoginRegistration").last?.message))
    XCTAssertEqual(request.executablePath, Bundle.main.executablePath)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.desktopSettings.autoLaunch = true
    payload.desktopSettings.crashReporting = true
    try http.publish(payload, target: "desktop-settings")
    payload = .init()
    payload.daemonInfo.cliInstallation.status = .allowed
    try http.publish(payload, target: "daemon-info")
    // Then
    try await eventuallyView(host) {
      host.element("settings-start-minimized")?.enabled == true
        && host.element("settings-auto-launch")?.value == "1"
        && host.element("settings-start-minimized")?.value == "0"
        && host.element("settings-performance-telemetry")?.value == "0"
        && host.element("settings-crash-reporting")?.value == "1"
        && host.element("settings-install-cli")?.enabled == true
    }
    XCTAssertNil(host.element("settings-cli-installation-reason"))
    await model.stop()
  }

  @MainActor func test_System操作_各トグルの保存拒否を表示して購読値に戻す() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.desktopSettings.autoLaunch = true }
    let model = try await testModel(http)
    let host = await ViewHarness(SystemSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-start-minimized")?.enabled == true }
    for (id, method, saved) in [
      ("settings-auto-launch", "UpdateLoginItemPreference", "1"),
      ("settings-start-minimized", "UpdateAppSettings", "0"),
      ("settings-performance-telemetry", "UpdatePerformanceTelemetry", "0"),
      ("settings-crash-reporting", "UpdateCrashReporting", "0"),
    ] {
      http.state.withLock { $0.failedMethod = method }
      // When
      try await host.press(id)
      // Then
      try await eventuallyView(host) {
        host.texts().contains("\(method) failed") && host.element(id)?.value == saved
      }
      XCTAssertEqual(http.requests(method).count, 1)
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("settings-crash-reporting")
    try await eventually { http.requests("UpdateCrashReporting").count == 2 }
    var payload = Releash_Client_V1_StatePayload()
    payload.desktopSettings.autoLaunch = true
    payload.desktopSettings.crashReporting = true
    try http.publish(payload, target: "desktop-settings")
    // Then
    try await eventuallyView(host) {
      host.element("settings-crash-reporting")?.value == "1"
        && host.element("settings-system-failure") == nil
    }
    await model.stop()
  }

  @MainActor func test_ログイン項目_承認待ちの表示とシステム設定への導線を持つ() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.loginRegistration.status = .readOnly
      $0.loginRegistration.reason = "アプリの場所は読み取り専用です"
    }
    let model = try await testModel(http)
    let state = SystemSettingsState()
    state.loginItemStatus = { .requiresApproval }
    var opened = false
    let host = await ViewHarness(
      SystemSettingsView(model: model, state: state, openLoginItems: { opened = true }))
    defer { host.close() }
    // Then
    try await eventuallyView(host) {
      host.texts().contains("ログイン項目は承認待ちです")
        && host.texts().contains("アプリの場所は読み取り専用です")
    }
    // When
    try await host.press("settings-open-login-items")
    // Then
    XCTAssertTrue(opened)
    // When
    state.loginItemStatus = { .enabled }
    state.refreshLoginItemStatus()
    // Then
    try await eventuallyView(host) { host.element("settings-open-login-items") == nil }
    XCTAssertFalse(host.texts().contains("ログイン項目は承認待ちです"))
    await model.stop()
  }

  @MainActor func test_CLI設置_成功と設置済みと失敗の理由を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.daemonInfo.cliInstallation.status = .allowed
      $0.cliInstallResult.path = "/usr/local/bin/releash"
    }
    let model = try await testModel(http)
    let host = await ViewHarness(SystemSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-install-cli")?.enabled == true }
    for (status, text): (Releash_Client_V1_CliInstallStatus, String) in [
      (.installed, "CLIを設置しました"), (.alreadyInstalled, "CLIは設置済みです"),
      (.unspecified, "CLIの設置結果を確認できません"),
    ] {
      http.state.withLock { $0.cliInstallResult.status = status }
      // When
      try await host.press("settings-install-cli")
      // Then
      try await eventuallyView(host) { host.texts().contains(text) }
      XCTAssertTrue(host.texts().contains("/usr/local/bin/releash"))
    }
    // When
    http.state.withLock { $0.failedMethod = "InstallCli" }
    try await host.press("settings-install-cli")
    // Then
    try await eventuallyView(host) { host.texts().contains("InstallCli failed") }
    XCTAssertFalse(host.texts().contains("CLIを設置しました"))
    XCTAssertFalse(host.texts().contains("CLIは設置済みです"))
    await model.stop()
  }
}
