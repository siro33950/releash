import AppKit
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class SettingsProvidersTests: XCTestCase {
  @MainActor func test_Providersタブ_購読から各Providerの可用性と実行ファイルを表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model))
    defer { host.close() }
    // When
    try await host.press("Providers")
    // Then
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/custom/claude"
        && host.element("settings-provider-executable-codex")?.value == ""
    }
    for text in [
      "Claude Code", "Codex", "利用可能", "利用不可", "既定の実行ファイル", "claude", "codex",
      "有効な実行ファイル", "/custom/claude", "解決された実行ファイル", "/resolved/claude",
      "利用できない理由", "not_found",
    ] {
      XCTAssertNotNil(host.element(text), text)
    }
    XCTAssertEqual(host.element("settings-provider-reset-claude")?.enabled, true)
    XCTAssertEqual(host.element("settings-provider-reset-codex")?.enabled, false)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = testProviderAvailability()
    payload.providerAvailability.providers.items[1].available = true
    payload.providerAvailability.providers.items[1].resolvedExecutable = "/resolved/codex"
    payload.providerAvailability.providers.items[1].clearUnavailableReason()
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventuallyView(host) {
      host.element("/resolved/codex") != nil && host.element("not_found") == nil
    }
    await model.stop()
  }

  @MainActor func test_上書き入力_確定時に保存し拒否時は理由と保存済みの値を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(ProvidersSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/custom/claude"
    }
    try await focusExecutable(host)
    http.state.withLock { $0.failedMethod = "UpdateProviderExecutable" }
    // When
    try await host.setTextField("/rejected/claude")
    // Then
    XCTAssertTrue(http.requests("UpdateProviderExecutable").isEmpty)
    // When
    try submit(host)
    // Then
    try await eventuallyView(host) {
      host.element("UpdateProviderExecutable failed") != nil
        && host.element("settings-provider-executable-claude")?.value == "/custom/claude"
    }
    XCTAssertEqual(http.requests("UpdateProviderExecutable").count, 1)
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await focusExecutable(host)
    try await host.setTextField("/new/claude")
    try submit(host)
    try await eventually { http.requests("UpdateProviderExecutable").count == 2 }
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = testProviderAvailability()
    payload.providerAvailability.providers.items[0].configuredExecutable = "/new/claude"
    payload.providerAvailability.providers.items[0].effectiveExecutable = "/new/claude"
    payload.providerAvailability.providers.items[0].resolvedExecutable = "/new/claude"
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/new/claude"
        && host.element("settings-provider-failure") == nil
    }
    host.close()
    // When
    let reopened = await ViewHarness(ProvidersSettingsView(model: model))
    defer { reopened.close() }
    // Then
    try await eventuallyView(reopened) {
      reopened.element("settings-provider-executable-claude")?.value == "/new/claude"
    }
    await model.stop()
  }

  @MainActor func test_リセットと再確認_専用RPCで保存し失敗の理由を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(ProvidersSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-provider-reset-claude")?.enabled == true
    }
    http.state.withLock { $0.failedMethod = "ResetProviderExecutable" }
    // When
    try await host.press("settings-provider-reset-claude")
    // Then
    try await eventuallyView(host) {
      host.element("ResetProviderExecutable failed") != nil
        && host.element("settings-provider-executable-claude")?.value == "/custom/claude"
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("settings-provider-reset-claude")
    try await eventually { http.requests("ResetProviderExecutable").count == 2 }
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = testProviderAvailability()
    payload.providerAvailability.providers.items[0].clearConfiguredExecutable()
    payload.providerAvailability.providers.items[0].effectiveExecutable = "claude"
    payload.providerAvailability.providers.items[0].resolvedExecutable = "/bin/claude"
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == ""
        && host.element("settings-provider-reset-claude")?.enabled == false
        && host.element("settings-provider-failure") == nil
    }
    // When
    http.state.withLock { $0.failedMethod = "RefreshProviderAvailability" }
    try await host.press("settings-provider-refresh")
    // Then
    try await eventuallyView(host) {
      host.element("RefreshProviderAvailability failed") != nil
        && host.element("settings-provider-refresh")?.enabled == true
    }
    XCTAssertEqual(http.requests("RefreshProviderAvailability").count, 1)
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("settings-provider-refresh")
    try await eventually { http.requests("RefreshProviderAvailability").count == 2 }
    payload.providerAvailability.providers.items[1].available = true
    payload.providerAvailability.providers.items[1].resolvedExecutable = "/bin/codex"
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventuallyView(host) {
      host.element("/bin/codex") != nil && host.element("settings-provider-failure") == nil
    }
    await model.stop()
  }

  @MainActor func test_上書き入力_フォーカスを離れると保存し切断中は操作できない() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(ProvidersSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.enabled == true
    }
    try await focusExecutable(host)
    try await host.setTextField("/new/claude")
    // When
    try await focusExecutable(host, index: 1)
    // Then
    try await eventually { http.requests("UpdateProviderExecutable").count == 1 }
    let request = try Releash_Client_V1_UpdateProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
    XCTAssertEqual(request.executable, "/new/claude")
    // When
    await model.stop()
    model.connected = false
    // Then
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.enabled == false
        && host.element("settings-provider-reset-claude")?.enabled == false
        && host.element("settings-provider-refresh")?.enabled == false
    }
  }

  @MainActor func test_リセット_編集中の下書きを上書き保存しない() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(ProvidersSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/custom/claude"
    }
    try await focusExecutable(host)
    try await host.setTextField("/draft/claude")
    // When
    try await host.press("settings-provider-reset-claude")
    // Then
    try await eventually { http.requests("ResetProviderExecutable").count == 1 }
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/custom/claude"
        && host.element("settings-provider-reset-claude")?.enabled == true
    }
    XCTAssertTrue(http.requests("UpdateProviderExecutable").isEmpty)
    await model.stop()
  }

  @MainActor func test_Providersの節を離れる_編集中の上書きを確定し購読を解放する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model))
    defer { host.close() }
    try await host.press("Providers")
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/custom/claude"
    }
    try await focusExecutable(host)
    try await host.setTextField("/closing/claude")
    let subscription = try XCTUnwrap(
      http.state.withLock { $0.subscriptions.last { $0.target == "provider-availability" } })
    // When
    try await host.press("General")
    // Then
    try await eventually { http.requests("UpdateProviderExecutable").count == 1 }
    let request = try Releash_Client_V1_UpdateProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
    XCTAssertEqual(request.executable, "/closing/claude")
    try await eventually {
      http.requests("StopStateSubscription").contains {
        let request = try? Releash_Client_V1_StopStateSubscriptionRequest(
          serializedBytes: $0.message ?? Data())
        return request?.subscriptionID == subscription.subscriptionID
      }
    }
    await model.stop()
  }

  @MainActor func test_別Providerの保存通知_未確定入力を保持して保存し拒否時に復元する() async throws {
    // Given
    let http = TestHTTPClient()
    var saved = testProviderAvailability()
    saved.providers.items.reverse()
    http.state.withLock { $0.providerAvailability = saved }
    let model = try await testModel(http)
    let host = await ViewHarness(ProvidersSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-provider-executable-codex")?.enabled == true
    }
    try await focusExecutable(host)
    try await host.setTextField("/draft/codex")
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = saved
    payload.providerAvailability.providers.items[1].configuredExecutable = "/new/claude"
    payload.providerAvailability.providers.items[1].effectiveExecutable = "/new/claude"
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventuallyView(host) {
      host.element("settings-provider-executable-claude")?.value == "/new/claude"
    }
    XCTAssertEqual(host.element("settings-provider-executable-codex")?.value, "/draft/codex")
    XCTAssertNotNil(host.element("codex"))
    XCTAssertTrue(http.requests("UpdateProviderExecutable").isEmpty)
    // When
    http.state.withLock { $0.failedMethod = "UpdateProviderExecutable" }
    try submit(host)
    // Then
    try await eventuallyView(host) {
      host.element("UpdateProviderExecutable failed") != nil
        && host.element("settings-provider-executable-codex")?.value == ""
    }
    let request = try Releash_Client_V1_UpdateProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
    XCTAssertEqual(request.provider, "codex")
    XCTAssertEqual(request.executable, "/draft/codex")
    XCTAssertEqual(host.element("settings-provider-executable-claude")?.value, "/new/claude")
    await model.stop()
  }

  @MainActor private func focusExecutable(_ host: ViewHarness, index: Int = 0) async throws {
    func fields(_ view: NSView) -> [NSTextField] {
      (view as? NSTextField).map { [$0] } ?? view.subviews.flatMap(fields)
    }
    let field = try XCTUnwrap(
      fields(try XCTUnwrap(host.window.contentView)).filter { $0.isEditable }.dropFirst(index).first
    )
    XCTAssertTrue(host.window.makeFirstResponder(field))
    try await Task.sleep(for: .milliseconds(50))
  }

  @MainActor private func submit(_ host: ViewHarness) throws {
    let enter = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, characters: "\r",
        charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
    host.window.sendEvent(enter)
  }
}
