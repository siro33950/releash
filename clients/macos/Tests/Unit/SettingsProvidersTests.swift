import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class SettingsProvidersTests: XCTestCase {
  @MainActor func test_Providers購読_可用性と実行ファイルを受け取り終了時に解放する() async throws {
    // Given
    let http = TestHTTPClient()
    let saved = testProviderAvailability()
    http.state.withLock { $0.providerAvailability = saved }
    let model = try await testModel(http)
    let state = ProvidersSettingsState()
    // When
    state.subscribe(model)
    state.subscribe(model)
    // Then
    try await eventually { state.snapshot == saved }
    XCTAssertEqual(state.executables, ["claude": "/custom/claude", "codex": ""])
    let subscriptions = http.state.withLock {
      $0.subscriptions.filter { $0.target == "provider-availability" }
    }
    XCTAssertEqual(subscriptions.count, 1)
    XCTAssertEqual(subscriptions.first?.args, [])
    // When
    var unrelated = Releash_Client_V1_StatePayload()
    unrelated.workflowConfig.approvalAutoApprove = true
    try http.publish(unrelated, target: "provider-availability")
    state.unsubscribe(model)
    state.unsubscribe(model)
    // Then
    try await eventually {
      http.requests("StopStateSubscription").contains {
        let request = try? Releash_Client_V1_StopStateSubscriptionRequest(
          serializedBytes: $0.message ?? Data())
        return request?.subscriptionID == subscriptions.first?.subscriptionID
      }
    }
    XCTAssertEqual(state.snapshot, saved)
    await model.stop()
  }

  @MainActor func test_上書き確定_RPCへ入力と認証を渡し購読値だけを表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.providerAvailability = testProviderAvailability() }
    let model = try await testModel(http)
    let state = ProvidersSettingsState()
    state.subscribe(model)
    try await eventually { state.snapshot != nil }
    // When
    state.executables["claude"] = "/new/claude"
    await state.updateExecutable("claude", model: model)
    // Then
    let request = try Releash_Client_V1_UpdateProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
    XCTAssertEqual(request.provider, "claude")
    XCTAssertEqual(request.executable, "/new/claude")
    XCTAssertEqual(
      http.requests("UpdateProviderExecutable").last?.headers["authorization"],
      ["Bearer operator-test"])
    XCTAssertEqual(state.snapshot?.providers.items.first?.effectiveExecutable, "/custom/claude")
    XCTAssertEqual(state.executables["claude"], "/custom/claude")
    XCTAssertNil(state.failure)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = testProviderAvailability()
    payload.providerAvailability.providers.items[0].configuredExecutable = "/server/claude"
    payload.providerAvailability.providers.items[0].effectiveExecutable = "/server/claude"
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventually { state.executables["claude"] == "/server/claude" }
    XCTAssertEqual(state.snapshot?.providers.items.first?.effectiveExecutable, "/server/claude")
    // When
    state.unsubscribe(model)
    let reopened = ProvidersSettingsState()
    reopened.subscribe(model)
    // Then
    try await eventually { reopened.executables["claude"] == "/server/claude" }
    reopened.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_リセットと再確認_専用RPCを呼び購読で既定と可用性を更新する() async throws {
    // Given
    let http = TestHTTPClient()
    let saved = testProviderAvailability()
    http.state.withLock { $0.providerAvailability = saved }
    let model = try await testModel(http)
    let state = ProvidersSettingsState()
    state.subscribe(model)
    try await eventually { state.snapshot != nil }
    // When
    await state.resetExecutable("claude", model: model)
    await state.refresh(model: model)
    // Then
    let reset = try Releash_Client_V1_ResetProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("ResetProviderExecutable").last?.message))
    XCTAssertEqual(reset.provider, "claude")
    _ = try Releash_Client_V1_RefreshProviderAvailabilityRequest(
      serializedBytes: XCTUnwrap(http.requests("RefreshProviderAvailability").last?.message))
    for method in ["ResetProviderExecutable", "RefreshProviderAvailability"] {
      XCTAssertEqual(http.requests(method).count, 1)
      XCTAssertEqual(
        http.requests(method).last?.headers["authorization"], ["Bearer operator-test"])
    }
    XCTAssertEqual(state.snapshot, saved)
    XCTAssertEqual(state.executables["claude"], "/custom/claude")
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = saved
    payload.providerAvailability.providers.items[0].clearConfiguredExecutable()
    payload.providerAvailability.providers.items[0].effectiveExecutable = "claude"
    payload.providerAvailability.providers.items[0].resolvedExecutable = "/bin/claude"
    payload.providerAvailability.providers.items[1].available = true
    payload.providerAvailability.providers.items[1].resolvedExecutable = "/bin/codex"
    payload.providerAvailability.providers.items[1].clearUnavailableReason()
    try http.publish(payload, target: "provider-availability")
    // Then
    try await eventually { state.snapshot == payload.providerAvailability }
    XCTAssertEqual(state.executables["claude"], "")
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_保存拒否_各RPCの理由を表示し保存済みの上書きを復元する() async throws {
    // Given
    let http = TestHTTPClient()
    let saved = testProviderAvailability()
    http.state.withLock { $0.providerAvailability = saved }
    let model = try await testModel(http)
    let state = ProvidersSettingsState()
    state.subscribe(model)
    try await eventually { state.snapshot != nil }
    for method in [
      "UpdateProviderExecutable", "ResetProviderExecutable", "RefreshProviderAvailability",
    ] {
      http.state.withLock { $0.failedMethod = method }
      // When
      switch method {
      case "UpdateProviderExecutable":
        state.executables["claude"] = ""
        await state.updateExecutable("claude", model: model)
      case "ResetProviderExecutable": await state.resetExecutable("claude", model: model)
      default: await state.refresh(model: model)
      }
      // Then
      XCTAssertEqual(state.failure, "\(method) failed")
      XCTAssertEqual(state.snapshot, saved)
      XCTAssertEqual(state.executables["claude"], "/custom/claude")
      XCTAssertTrue(state.saving.isEmpty)
    }
    let empty = try Releash_Client_V1_UpdateProviderExecutableRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
    XCTAssertTrue(empty.hasExecutable)
    XCTAssertEqual(empty.executable, "")
    // When
    http.state.withLock { $0.failedMethod = nil }
    await state.refresh(model: model)
    // Then
    XCTAssertNil(state.failure)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_上書き_未受信と未変更は送信せず未接続時は理由を表示する() async {
    // Given
    let state = ProvidersSettingsState()
    let model = AppModel()
    state.executables["claude"] = "draft"
    // When
    await state.updateExecutable("claude", model: model)
    // Then
    XCTAssertNil(state.failure)
    // When
    state.snapshot = testProviderAvailability()
    state.executables["claude"] = "/custom/claude"
    await state.updateExecutable("claude", model: model)
    await state.updateExecutable("unknown", model: model)
    // Then
    XCTAssertNil(state.failure)
    // When
    state.executables["claude"] = "draft"
    await state.updateExecutable("claude", model: model)
    // Then
    XCTAssertEqual(state.failure, "サーバに接続されていません")
    XCTAssertEqual(state.executables["claude"], "/custom/claude")
    XCTAssertTrue(state.saving.isEmpty)
  }

  @MainActor func test_保存中_同じProviderの重複確定を送らず購読の更新を保持する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.providerAvailability = testProviderAvailability()
      $0.deferSave = true
    }
    let model = try await testModel(http)
    let state = ProvidersSettingsState()
    state.subscribe(model)
    try await eventually { state.snapshot != nil }
    state.executables["claude"] = "/new/claude"
    // When
    let save = Task { await state.updateExecutable("claude", model: model) }
    try await eventually { http.state.withLock { $0.pendingSave != nil } }
    await state.updateExecutable("claude", model: model)
    // Then
    XCTAssertEqual(http.requests("UpdateProviderExecutable").count, 1)
    XCTAssertEqual(state.saving, [.executable("claude")])
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.providerAvailability = testProviderAvailability()
    payload.providerAvailability.providers.items[0].configuredExecutable = "/new/claude"
    try http.publish(payload, target: "provider-availability")
    try await eventually { state.executables["claude"] == "/new/claude" }
    let respond = http.state.withLock { $0.pendingSave }
    try XCTUnwrap(respond)(
      HTTPResponse(
        code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
        trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    await save.value
    // Then
    XCTAssertEqual(state.executables["claude"], "/new/claude")
    XCTAssertTrue(state.saving.isEmpty)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_別Providerの保存通知_未確定入力を保持し確定値の送信と拒否時の復元を行う() async throws {
    for rejected in [false, true] {
      // Given
      let http = TestHTTPClient()
      http.state.withLock {
        $0.providerAvailability = testProviderAvailability()
        $0.deferSave = true
      }
      let model = try await testModel(http)
      let state = ProvidersSettingsState()
      state.subscribe(model)
      try await eventually { state.snapshot != nil }
      state.executables["claude"] = "/new/claude"
      let save = Task { await state.updateExecutable("claude", model: model) }
      try await eventually { http.state.withLock { $0.pendingSave != nil } }
      state.executables["codex"] = "/draft/codex"
      // When
      var payload = Releash_Client_V1_StatePayload()
      payload.providerAvailability = testProviderAvailability()
      payload.providerAvailability.providers.items[0].configuredExecutable = "/new/claude"
      payload.providerAvailability.providers.items[0].effectiveExecutable = "/new/claude"
      payload.providerAvailability.providers.items[1].available = true
      payload.providerAvailability.providers.items[1].resolvedExecutable = "/resolved/codex"
      payload.providerAvailability.providers.items[1].clearUnavailableReason()
      try http.publish(payload, target: "provider-availability")
      // Then
      try await eventually { state.snapshot == payload.providerAvailability }
      XCTAssertEqual(state.executables["codex"], "/draft/codex")
      XCTAssertEqual(state.snapshot?.providers.items[1].effectiveExecutable, "codex")
      XCTAssertEqual(state.snapshot?.providers.items[1].resolvedExecutable, "/resolved/codex")
      // When
      let respond = http.state.withLock { $0.pendingSave }
      try XCTUnwrap(respond)(
        HTTPResponse(
          code: .ok, headers: ["content-type": ["application/proto"]], message: Data(),
          trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
      await save.value
      http.state.withLock {
        $0.deferSave = false
        $0.failedMethod = rejected ? "UpdateProviderExecutable" : nil
      }
      await state.updateExecutable("codex", model: model)
      // Then
      let request = try Releash_Client_V1_UpdateProviderExecutableRequest(
        serializedBytes: XCTUnwrap(http.requests("UpdateProviderExecutable").last?.message))
      XCTAssertEqual(http.requests("UpdateProviderExecutable").count, 2)
      XCTAssertEqual(request.provider, "codex")
      XCTAssertEqual(request.executable, "/draft/codex")
      XCTAssertEqual(state.executables["claude"], "/new/claude")
      XCTAssertEqual(state.executables["codex"], "")
      XCTAssertEqual(state.failure, rejected ? "UpdateProviderExecutable failed" : nil)
      XCTAssertTrue(state.saving.isEmpty)
      if !rejected {
        // When
        payload.providerAvailability.providers.items[1].configuredExecutable = "/draft/codex"
        payload.providerAvailability.providers.items[1].effectiveExecutable = "/draft/codex"
        try http.publish(payload, target: "provider-availability")
        // Then
        try await eventually { state.executables["codex"] == "/draft/codex" }
      }
      state.unsubscribe(model)
      await model.stop()
    }
  }
}
