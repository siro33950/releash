import SwiftUI
import XCTest

@testable import Releash

final class SettingsWorkflowsTests: XCTestCase {
  @MainActor func test_Workflows一覧_名前と印を表示し購読更新に追従する() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let host = await ViewHarness(SettingsView(model: model), size: NSSize(width: 1024, height: 800))
    defer { host.close() }
    // When
    try await host.press("Workflows")
    // Then
    try await eventuallyView(host) {
      host.element("settings-workflow-01_author-spec")?.label
        == "01_author-spec, 実行中, error: 1"
    }
    XCTAssertEqual(
      host.element("settings-workflow-builtin-flow")?.label,
      "builtin-flow, builtin")
    XCTAssertEqual(host.element("settings-workflow-broken")?.label, "broken")
    XCTAssertNotNil(host.element("error: 0"))
    XCTAssertNotNil(host.element("info: 0"))
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workflows = http.state.withLock { $0.workflows }
    payload.workflows.items[1].description_p = "一覧の更新"
    payload.workflows.items[1].builtin = true
    try http.publish(payload, target: "workflows")
    payload = .init()
    payload.diagnostics = http.state.withLock { $0.diagnostics }
    payload.diagnostics.workflowSummaries.entries["01_author-spec"]?.errorCount = 7
    try http.publish(payload, target: "diagnostics")
    // Then
    try await eventuallyView(host) {
      host.element("settings-workflow-01_author-spec")?.label
        == "01_author-spec, builtin, 実行中, error: 7"
    }
    // When
    http.state.withLock { $0.failedMethod = "OpenWorkflowDirectoryInEditor" }
    try await host.press("settings-open-workflows-directory")
    // Then
    try await eventuallyView(host) { host.texts().contains("OpenWorkflowDirectoryInEditor failed") }
    XCTAssertNotNil(host.element("settings-workflows-failure"))
    model.connected = false
    try await eventuallyView(host) {
      host.element("settings-open-workflows-directory")?.enabled == false
    }
    await model.stop()
  }

  @MainActor func test_定義詳細_Nodeの内容と診断を表示し購読更新とFacetへの往復に追従する() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let host = await ViewHarness(WorkflowsSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-workflow-01_author-spec") != nil }
    // When
    try await host.press("settings-workflow-01_author-spec")
    // Then
    try await eventuallyView(host) {
      host.element("settings-workflow-node-review") != nil
        && host.element("settings-diagnostic-WFE001") != nil
    }
    for text in ["利用者の定義", "WFE001 / error / parse_shape", "01_author-spec.lua:2:3–4:5", "定義のエラー"] {
      XCTAssertTrue(host.texts().contains(text), text + host.debugDump())
    }
    XCTAssertNil(host.element("settings-diagnostic-WFI000"))
    XCTAssertNotNil(host.element("settings-open-workflow-in-editor"))
    // When
    try await host.press("settings-workflow-node-review")
    // Then
    try await eventuallyView(host) { host.texts().contains("test-model") }
    for text in [
      "codex", "read-only", "policy: coding", "knowledge: guide", "instruction: implement",
      "report", "source", "ReviewInput", "approval", "isolated",
    ] {
      XCTAssertTrue(host.texts().contains(text), text + host.debugDump())
    }
    XCTAssertTrue(
      host.texts().contains(where: {
        $0.contains("worker") && $0.contains("retry") && $0.contains("input.task")
      }))
    for (name, text) in [("check", "echo checked"), ("parallel", "worker"), ("serial", "first")] {
      try await host.press("settings-workflow-node-\(name)")
      try await eventuallyView(host) { host.texts().contains(text) }
    }
    XCTAssertTrue(host.texts().contains(where: { $0.contains("tasks") }))
    // When
    http.state.withLock { $0.failedMethod = "OpenWorkflowInEditor" }
    try await host.press("settings-open-workflow-in-editor")
    // Then
    try await eventuallyView(host) { host.texts().contains("OpenWorkflowInEditor failed") }
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workflow = http.state.withLock { $0.workflowDetails["01_author-spec"]! }
    payload.workflow.value.description_p = "詳細の更新"
    payload.workflow.value.builtin = true
    try http.publish(payload, target: "workflow", args: ["01_author-spec"])
    payload = .init()
    payload.diagnostics = http.state.withLock { $0.diagnostics }
    payload.diagnostics.items.items[0].message = "診断の更新"
    payload.diagnostics.items.items[0].stage.value = .UNRECOGNIZED(99)
    try http.publish(payload, target: "diagnostics")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("詳細の更新") && host.texts().contains("診断の更新")
        && host.texts().contains("WFE001 / error / 99")
        && host.element("settings-open-workflow-in-editor") == nil
    }
    // When
    try await host.press("policy: coding")
    // Then
    try await eventuallyView(host) {
      host.element("settings-facet-usage-01_author-spec-review") != nil
    }
    XCTAssertTrue(host.texts().contains("共有Facetの説明"))
    XCTAssertNotNil(host.element("settings-facet-usage-builtin-flow-review"))
    XCTAssertNotNil(host.element("settings-diagnostic-WFE003"))
    XCTAssertNil(host.element("settings-diagnostic-WFE001"))
    XCTAssertNotNil(host.element("settings-open-facet-in-editor"))
    XCTAssertFalse(
      http.state.withLock {
        $0.subscriptions.contains { $0.target == "facet" || $0.target == "workflow-source" }
      })
    // When
    http.state.withLock { $0.failedMethod = "OpenFacetInEditor" }
    try await host.press("settings-open-facet-in-editor")
    // Then
    try await eventuallyView(host) { host.texts().contains("OpenFacetInEditor failed") }
    // When
    payload = .init()
    payload.facets = http.state.withLock { $0.facets["policy"]! }
    payload.facets.items[1].description_p = "Facetの更新"
    payload.facets.items[1].builtin = true
    try http.publish(payload, target: "facets", args: ["policy"])
    // Then
    try await eventuallyView(host) {
      host.texts().contains("Facetの更新") && host.element("settings-open-facet-in-editor") == nil
    }
    // When
    try await host.press("settings-facet-usage-builtin-flow-review")
    // Then
    try await eventuallyView(host) { host.element("settings-workflow-node-review") != nil }
    XCTAssertTrue(host.texts().contains("組み込みの定義"))
    XCTAssertTrue(host.texts().contains("組み込みの情報"))
    XCTAssertNil(host.element("settings-open-workflow-in-editor"))
    await model.stop()
  }

  @MainActor func test_詳細間の移動後_戻るボタンで一覧に戻る() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let host = await ViewHarness(WorkflowsSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-workflow-01_author-spec") != nil }
    // When
    try await host.press("settings-workflow-01_author-spec")
    try await eventuallyView(host) { host.element("settings-workflow-node-review") != nil }
    try await host.press("settings-workflow-node-review")
    try await eventuallyView(host) { host.element("policy: coding") != nil }
    try await host.press("policy: coding")
    try await eventuallyView(host) {
      host.element("settings-facet-usage-01_author-spec-review") != nil
    }
    try await host.press("戻る")
    // Then
    try await eventuallyView(host) { host.element("settings-workflow-01_author-spec") != nil }
    // When
    try await host.press("Facet")
    try await eventuallyView(host) { host.element("settings-facet-policy-coding") != nil }
    try await host.press("settings-facet-policy-coding")
    try await eventuallyView(host) {
      host.element("settings-facet-usage-01_author-spec-review") != nil
    }
    try await host.press("settings-facet-usage-01_author-spec-review")
    try await eventuallyView(host) { host.element("settings-workflow-node-review") != nil }
    try await host.press("戻る")
    // Then
    try await eventuallyView(host) { host.element("settings-facet-policy-coding") != nil }
    await model.stop()
  }

  @MainActor func test_解釈できない定義_診断だけを表示する() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let host = await ViewHarness(WorkflowsSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-workflow-broken") != nil }
    // When
    try await host.press("settings-workflow-broken")
    // Then
    try await eventuallyView(host) { host.element("settings-diagnostic-WFE002") != nil }
    XCTAssertTrue(host.texts().contains("解釈できない定義"))
    XCTAssertNil(host.element("settings-open-workflow-in-editor"))
    XCTAssertNil(host.element("settings-workflow-node-review"))
    XCTAssertFalse(host.texts().contains(where: { $0.contains("01_author-spec.lua:") }))
    await model.stop()
  }

  @MainActor func test_Facet一覧_種類を切り替え各欄と件数を表示しbuiltinには編集操作がない() async throws {
    // Given
    let http = TestHTTPClient()
    testWorkflowSettings(http)
    let model = try await testModel(http)
    let host = await ViewHarness(WorkflowsSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-workflows-mode") != nil }
    // When
    try await host.press("Facet")
    // Then
    try await eventuallyView(host) {
      host.element("settings-facet-policy-coding")?.label == "coding, error: 3"
    }
    XCTAssertEqual(
      host.element("settings-facet-policy-builtin-facet")?.label,
      "builtin-facet, builtin")
    for kind in ["knowledge", "instruction", "policy"] {
      // When
      try await host.press(kind)
      // Then
      try await eventuallyView(host) { host.element("settings-facet-\(kind)-coding") != nil }
    }
    // When
    try await host.press("settings-facet-policy-builtin-facet")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("組み込みFacet") && host.element("settings-facet-kind") == nil
    }
    XCTAssertNil(host.element("settings-open-facet-in-editor"))
    await model.stop()
  }
}
