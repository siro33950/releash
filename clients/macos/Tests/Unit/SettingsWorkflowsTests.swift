import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class SettingsWorkflowsTests: XCTestCase {
  @MainActor func test_Workflows購読_選択した引数で購読し更新を受け終了時に解放する() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let state = WorkflowsSettingsState()
    // When
    state.subscribe(model)
    state.subscribe(model)
    state.selectWorkflow("01_author-spec", model)
    state.selectFacetKind("policy", model)
    // Then
    try await eventually {
      state.workflows.items.count == 3 && state.workflow?.value.name == "01_author-spec"
        && state.facets.items.count == 4 && state.diagnostics.items.items.count == 4
    }
    let targets = ["workflows", "workflow", "facets", "diagnostics"]
    let subscriptions = http.state.withLock {
      $0.subscriptions.filter { targets.contains($0.target) }
    }
    XCTAssertEqual(subscriptions.count, 4)
    for subscription in subscriptions {
      XCTAssertEqual(
        subscription.args,
        subscription.target == "workflow"
          ? ["01_author-spec"] : subscription.target == "facets" ? ["policy"] : [])
    }
    // When
    state.selectWorkflow("broken", model)
    state.selectFacetKind("knowledge", model)
    // Then
    try await eventually {
      state.workflow != nil && state.workflow?.hasValue == false
        && state.facets.items.first?.kind == "knowledge"
    }
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workflow.value.name = "broken"
    payload.workflow.value.description_p = "修正済み"
    try http.publish(payload, target: "workflow", args: ["broken"])
    // Then
    try await eventually { state.workflow?.value.description_p == "修正済み" }
    // When
    state.unsubscribe(model)
    // Then
    try await eventually {
      let stopped = http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      }
      return http.state.withLock {
        $0.subscriptions.filter { targets.contains($0.target) && $0.target != "workflows" }
          .allSatisfy {
            stopped.contains($0.subscriptionID)
          }
      }
    }
    let saved = state.workflows
    payload = .init()
    payload.workflows.items = []
    try http.publish(payload, target: "workflows")
    try await eventually { model.workflows.isEmpty }
    XCTAssertEqual(state.workflows, saved)
    state.subscribe(model)
    try await eventually { state.workflows.items.isEmpty }
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_外部エディタ_各RPCの引数と認証を送り失敗を保持する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = WorkflowsSettingsState()
    // When
    await state.openWorkflow("01_author-spec", model)
    await state.openFacet("knowledge", "guide", model)
    await state.openDirectory(model)
    // Then
    let workflow = try Releash_Client_V1_OpenWorkflowInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenWorkflowInEditor").last?.message))
    XCTAssertEqual(workflow.name, "01_author-spec")
    let facet = try Releash_Client_V1_OpenFacetInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenFacetInEditor").last?.message))
    XCTAssertEqual(facet.kind, "knowledge")
    XCTAssertEqual(facet.key, "guide")
    let directory = try Releash_Client_V1_OpenWorkflowDirectoryInEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("OpenWorkflowDirectoryInEditor").last?.message))
    XCTAssertEqual(try directory.serializedData(), Data())
    for method in ["OpenWorkflowInEditor", "OpenFacetInEditor", "OpenWorkflowDirectoryInEditor"] {
      XCTAssertEqual(http.requests(method).last?.headers["authorization"], ["Bearer operator-test"])
      // When
      http.state.withLock { $0.failedMethod = method }
      switch method {
      case "OpenWorkflowInEditor": await state.openWorkflow("01_author-spec", model)
      case "OpenFacetInEditor": await state.openFacet("policy", "coding", model)
      default: await state.openDirectory(model)
      }
      // Then
      XCTAssertEqual(state.failure, "\(method) failed")
      XCTAssertTrue(state.saving.isEmpty)
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    await state.openDirectory(model)
    // Then
    XCTAssertNil(state.failure)
    await model.stop()
    await state.openDirectory(AppModel())
    XCTAssertEqual(state.failure, "サーバに接続されていません")
  }
}
