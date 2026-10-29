import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class SettingsRepositoriesTests: XCTestCase {
  @MainActor func test_Repository購読_パスごとの保存値と変更を受け取り終了時に解放する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.releashBases["/repo"] = .with { $0.value = "develop" }
      $0.branches["/repo"] = .with { $0.items = [.with { $0.name = "develop" }] }
      $0.notionConfigs["/repo"] = .with {
        $0.value.apiToken = "saved-token"
        $0.value.databaseID = "saved-database"
        $0.value.propertyMapping.title = "Name"
      }
    }
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    // When
    state.subscribe(model)
    state.subscribe(model)
    // Then
    try await eventually {
      state.base?.value == "develop" && state.branches?.items.first?.name == "develop"
        && state.apiToken == "saved-token"
    }
    XCTAssertEqual(state.databaseID, "saved-database")
    XCTAssertEqual(state.mapping.title, "Name")
    let targets = ["releash-base", "branches", "notion-config"]
    let subscriptions = http.state.withLock {
      $0.subscriptions.filter { targets.contains($0.target) }
    }
    XCTAssertEqual(subscriptions.count, targets.count)
    XCTAssertTrue(subscriptions.allSatisfy { $0.args == ["/repo"] })
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.notionConfig.value.apiToken = "other-repo"
    try http.publish(payload, target: "notion-config", args: ["/other"])
    payload.notionConfig.value.apiToken = "updated"
    try http.publish(payload, target: "notion-config", args: ["/repo"])
    // Then
    try await eventually { state.apiToken == "updated" }
    // When
    state.unsubscribe(model)
    // Then
    try await eventually {
      let stopped = http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      }
      return subscriptions.allSatisfy { stopped.contains($0.subscriptionID) }
    }
    // When
    let reopened = RepositorySettingsState(path: "/repo")
    reopened.subscribe(model)
    // Then
    try await eventually { reopened.apiToken == "updated" }
    reopened.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_BaseBranch_確定値と自動をRPCへ渡し表示は購読で更新する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.releashBases["/repo"] = .with { $0.value = "main" } }
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    state.subscribe(model)
    try await eventually { state.base?.value == "main" }
    // When
    await state.updateBase("develop", model: model)
    // Then
    let selected = try Releash_Client_V1_SetReleashBaseRequest(
      serializedBytes: XCTUnwrap(http.requests("SetReleashBase").last?.message))
    XCTAssertEqual(selected.repoPath, "/repo")
    XCTAssertEqual(selected.base, "develop")
    XCTAssertEqual(state.base?.value, "main")
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.releashBase.value = "develop"
    try http.publish(payload, target: "releash-base", args: ["/repo"])
    // Then
    try await eventually { state.base?.value == "develop" }
    // When
    await state.updateBase("", model: model)
    // Then
    let automatic = try Releash_Client_V1_SetReleashBaseRequest(
      serializedBytes: XCTUnwrap(http.requests("SetReleashBase").last?.message))
    XCTAssertEqual(automatic.repoPath, "/repo")
    XCTAssertFalse(automatic.hasBase)
    XCTAssertEqual(state.base?.value, "develop")
    // When
    payload.releashBase = .init()
    try http.publish(payload, target: "releash-base", args: ["/repo"])
    // Then
    try await eventually { state.base?.hasValue == false }
    XCTAssertNil(state.failure)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_Notion接続テスト_全ての結果とpropertyを表示し入力変更で成功を無効にする() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    state.apiToken = "token"
    state.databaseID = "database"
    let cases: [(Releash_Client_V1_NotionConfigStatusView.Value, String)] = [
      (.notConfigured, "未設定です"), (.configured, "接続に成功しました"),
      (.invalidToken, "API tokenが不正です"), (.invalidDatabase, "Database IDが不正です"),
      (.networkError, "ネットワークに接続できません"),
      (.UNRECOGNIZED(99), "接続テストの結果を確認できません"),
    ]
    for (status, message) in cases {
      http.state.withLock {
        $0.notionValidation.status.value = status
        $0.notionValidation.properties.items = [
          .with {
            $0.name = "Name"
            $0.propertyType = "title"
          }
        ]
      }
      // When
      await state.validateNotion(model: model)
      // Then
      XCTAssertEqual(state.validationMessage, message)
      XCTAssertEqual(state.validation?.properties.items.first?.name, "Name")
      XCTAssertNil(state.failure)
    }
    let request = try Releash_Client_V1_ValidateNotionConfigRequest(
      serializedBytes: XCTUnwrap(http.requests("ValidateNotionConfig").last?.message))
    XCTAssertEqual(request.apiToken, "token")
    XCTAssertEqual(request.databaseID, "database")
    for field in ["token", "database"] {
      http.state.withLock { $0.notionValidation.status.value = .configured }
      await state.validateNotion(model: model)
      // When
      if field == "token" { state.apiToken = "changed" } else { state.databaseID = "changed" }
      await state.saveNotion(model: model)
      // Then
      XCTAssertNil(state.validation)
      XCTAssertTrue(http.requests("SaveNotionConfig").isEmpty)
    }
    await model.stop()
  }

  @MainActor func test_Notion保存と削除_入力の対応を送信し購読だけを保存値にする() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.notionValidation.status.value = .configured }
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    state.subscribe(model)
    try await eventually { state.notion != nil }
    state.apiToken = "new-token"
    state.databaseID = "new-database"
    await state.validateNotion(model: model)
    state.mapping.title = "Name"
    state.mapping.labels.items = [
      .with {
        $0.name = "Status"
        $0.propertyType = "status"
      }
    ]
    state.mapping.branchName = "Branch"
    state.mapping.branchPrefix = "feat/"
    // When
    await state.saveNotion(model: model)
    // Then
    let request = try Releash_Client_V1_SaveNotionConfigRequest(
      serializedBytes: XCTUnwrap(http.requests("SaveNotionConfig").last?.message))
    XCTAssertEqual(request.repoPath, "/repo")
    XCTAssertEqual(request.apiToken, "new-token")
    XCTAssertEqual(request.databaseID, "new-database")
    XCTAssertEqual(request.propertyMapping.title, "Name")
    XCTAssertEqual(request.propertyMapping.labels.items.first?.name, "Status")
    XCTAssertEqual(request.propertyMapping.labels.items.first?.propertyType, "status")
    XCTAssertEqual(request.propertyMapping.branchName, "Branch")
    XCTAssertEqual(request.propertyMapping.branchPrefix, "feat/")
    XCTAssertEqual(state.notion?.hasValue, false)
    XCTAssertEqual(state.apiToken, "")
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.notionConfig.value.apiToken = request.apiToken
    payload.notionConfig.value.databaseID = request.databaseID
    payload.notionConfig.value.propertyMapping = request.propertyMapping
    try http.publish(payload, target: "notion-config", args: ["/repo"])
    // Then
    try await eventually { state.apiToken == "new-token" && state.mapping.branchPrefix == "feat/" }
    // When
    await state.deleteNotion(model: model)
    // Then
    let deletion = try Releash_Client_V1_DeleteNotionConfigRequest(
      serializedBytes: XCTUnwrap(http.requests("DeleteNotionConfig").last?.message))
    XCTAssertEqual(deletion.repoPath, "/repo")
    XCTAssertEqual(state.notion?.hasValue, true)
    // When
    payload.notionConfig = .init()
    try http.publish(payload, target: "notion-config", args: ["/repo"])
    // Then
    try await eventually { state.notion?.hasValue == false && state.apiToken.isEmpty }
    for method in ["ValidateNotionConfig", "SaveNotionConfig", "DeleteNotionConfig"] {
      XCTAssertEqual(http.requests(method).last?.headers["authorization"], ["Bearer operator-test"])
    }
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_Repository削除_一覧とサイドバーはworkspacesの通知で更新する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.repositories = [.with { $0.path = "/repo" }] }
    let model = try await testModel(http)
    try await eventually { model.repositories.count == 1 }
    let state = RepositorySettingsState(path: "/repo")
    // When
    await state.removeRepository(model: model)
    // Then
    let request = try Releash_Client_V1_RemoveRepoPathRequest(
      serializedBytes: XCTUnwrap(http.requests("RemoveRepoPath").last?.message))
    XCTAssertEqual(request.path, "/repo")
    XCTAssertEqual(
      http.requests("RemoveRepoPath").last?.headers["authorization"], ["Bearer operator-test"])
    XCTAssertEqual(model.repositories.map(\.path), ["/repo"])
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = []
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventually { model.repositories.isEmpty }
    await model.stop()
  }

  @MainActor func test_Repository保存失敗_理由を表示して保存済みの値と入力を復元する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.releashBases["/repo"] = .with { $0.value = "main" }
      $0.notionConfigs["/repo"] = .with {
        $0.value.apiToken = "saved-token"
        $0.value.databaseID = "saved-database"
        $0.value.propertyMapping.title = "Saved"
      }
      $0.notionValidation.status.value = .configured
    }
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    state.subscribe(model)
    try await eventually { state.apiToken == "saved-token" && state.base?.value == "main" }
    for method in [
      "SetReleashBase", "SaveNotionConfig", "DeleteNotionConfig", "RemoveRepoPath",
      "ValidateNotionConfig",
    ] {
      http.state.withLock { $0.failedMethod = nil }
      state.apiToken = "rejected"
      await state.validateNotion(model: model)
      state.mapping.title = "Rejected"
      http.state.withLock { $0.failedMethod = method }
      // When
      switch method {
      case "SetReleashBase": await state.updateBase("rejected", model: model)
      case "SaveNotionConfig": await state.saveNotion(model: model)
      case "DeleteNotionConfig": await state.deleteNotion(model: model)
      case "RemoveRepoPath": await state.removeRepository(model: model)
      default: await state.validateNotion(model: model)
      }
      // Then
      XCTAssertEqual(state.failure, "\(method) failed")
      XCTAssertEqual(state.base?.value, "main")
      XCTAssertEqual(state.notion?.value.apiToken, "saved-token")
      if method == "SaveNotionConfig" || method == "DeleteNotionConfig" {
        XCTAssertEqual(state.apiToken, "saved-token")
        XCTAssertEqual(state.databaseID, "saved-database")
        XCTAssertEqual(state.mapping.title, "Saved")
        XCTAssertNil(state.validation)
      }
      XCTAssertTrue(state.saving.isEmpty)
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    await state.updateBase("develop", model: model)
    // Then
    XCTAssertNil(state.failure)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_Notion検証中_操作を重複させず別の入力に届いた成功を表示しない() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.deferNotionValidation = true }
    let model = try await testModel(http)
    let state = RepositorySettingsState(path: "/repo")
    state.apiToken = "old-token"
    state.databaseID = "database"
    let validation = Task { await state.validateNotion(model: model) }
    try await eventually { http.state.withLock { $0.pendingNotionValidation != nil } }
    // When
    await state.validateNotion(model: model)
    await state.saveNotion(model: model)
    await state.deleteNotion(model: model)
    state.apiToken = "new-token"
    let result = Releash_Client_V1_NotionValidationResultView.with { $0.status.value = .configured }
    let completion = try XCTUnwrap(http.state.withLock { $0.pendingNotionValidation })
    completion(
      HTTPResponse(
        code: .ok, headers: ["content-type": ["application/proto"]],
        message: try result.serializedData(), trailers: [:], error: nil,
        tracingInfo: .init(httpStatus: 200)))
    await validation.value
    // Then
    XCTAssertEqual(http.requests("ValidateNotionConfig").count, 1)
    XCTAssertTrue(http.requests("SaveNotionConfig").isEmpty)
    XCTAssertTrue(http.requests("DeleteNotionConfig").isEmpty)
    XCTAssertNil(state.validation)
    XCTAssertTrue(state.saving.isEmpty)
    await model.stop()
  }

  @MainActor func test_Repository未接続_保存せず理由を表示してNotion入力を復元する() async {
    // Given
    let model = AppModel()
    let state = RepositorySettingsState(path: "/repo")
    state.apiToken = "draft"
    state.validation = .with { $0.status.value = .configured }
    // When
    await state.saveNotion(model: model)
    // Then
    XCTAssertEqual(state.failure, "サーバに接続されていません")
    XCTAssertEqual(state.apiToken, "")
    XCTAssertTrue(state.saving.isEmpty)
  }
}
