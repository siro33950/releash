import SwiftUI
import XCTest

@testable import Releash

final class NodeInspectorTests: XCTestCase {
  @MainActor func testInspectorActionsTargetSelectedNodeAndDisappearWhenCapabilitiesChange()
    async throws
  {
    let http = TestHTTPClient()
    var detail = Releash_Client_V1_WorkspaceNodeDetailDto()
    detail.id = "selected-child"
    detail.title = "Selected child"
    detail.executionID = "selected-execution"
    detail.errorReason = "Selected node failed"
    detail.content.command = .init()
    detail.capabilities.canRetry = true
    detail.canAbort = true
    http.state.withLock { $0.nodeDetails[detail.id] = detail }
    let model = try await testModel(http)
    let host = await ViewHarness(
      NodeInspector(model: model, path: "/selected-worktree", nodeID: detail.id))
    defer { host.close() }
    try await eventuallyView(host) { host.element("Retry") != nil && host.element("Abort") != nil }
    XCTAssertTrue(host.texts().contains("Selected node failed"))
    try await host.press("Retry")
    try await host.press("Abort")
    try await eventually {
      !http.requests("RetryWorkspaceNode").isEmpty && !http.requests("AbortWorkflow").isEmpty
    }
    let retry = try Releash_Client_V1_RetryWorkspaceNodeRequest(
      serializedBytes: XCTUnwrap(http.requests("RetryWorkspaceNode").last?.message))
    XCTAssertEqual(retry.worktreePath, "/selected-worktree")
    XCTAssertEqual(retry.nodeID, "selected-child")
    let abort = try Releash_Client_V1_AbortWorkflowRequest(
      serializedBytes: XCTUnwrap(http.requests("AbortWorkflow").last?.message))
    XCTAssertEqual(abort.executionID, "selected-execution")
    detail.clearErrorReason()
    detail.capabilities.canRetry = false
    detail.canAbort = false
    var payload = Releash_Client_V1_StatePayload()
    payload.nodeDetail.value = detail
    try http.publish(payload, target: "node-detail")
    try await eventuallyView(host) { host.element("Retry") == nil && host.element("Abort") == nil }
    XCTAssertFalse(host.texts().contains("Selected node failed"))
    await model.stop()
  }

  @MainActor func testInspectorSubscribesToSelectedExecutionAndReleasesSubscriptions() async throws
  {
    let http = TestHTTPClient()
    var first = Releash_Client_V1_WorkspaceNodeDetailDto()
    first.id = "first"
    first.title = "Human session"
    first.nodeName = "loop"
    first.nodeExecutionID = "visit-1"
    first.model = "model-1"
    var second = first
    second.id = "second"
    second.nodeExecutionID = "visit-2"
    var one = Releash_Client_V1_WorkflowOutputView()
    one.submitted.structuredOutput.stringValue.value = "first artifact"
    var two = one
    two.submitted.structuredOutput.stringValue.value = "second artifact"
    http.state.withLock {
      $0.nodeDetails = ["first": first, "second": second]
      $0.artifacts = ["visit-1": one, "visit-2": two]
    }
    let model = try await testModel(http)
    for (id, artifact) in [("first", "first artifact"), ("second", "second artifact")] {
      let host = await ViewHarness(AnyView(NodeInspector(model: model, path: "/repo", nodeID: id)))
      try await eventuallyView(host) { host.texts().contains(artifact) }
      XCTAssertTrue(host.texts().contains("Human session"))
      XCTAssertTrue(host.texts().contains("model-1"))
      try await host.press("artifact-json")
      try await eventuallyView(host) { host.texts().contains("\"" + artifact + "\"") }
      XCTAssertNil(host.element("Retry"))
      XCTAssertNil(host.element("Abort"))
      let subscriptions = http.state.withLock { state in
        state.subscriptions.filter {
          ($0.target == "node-detail" && $0.args == ["/repo", id]) || $0.target == "node-artifact"
        }.map(\.subscriptionID)
      }
      let hostingView = try XCTUnwrap(host.window.contentView as? NSHostingView<AnyView>)
      hostingView.rootView = AnyView(EmptyView())
      try await eventually {
        let stopped = http.requests("StopStateSubscription").compactMap {
          try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
            .subscriptionID
        }
        return subscriptions.allSatisfy { stopped.contains($0) }
      }
      host.close()
    }
    let ids = http.state.withLock {
      $0.subscriptions.filter { $0.target == "node-artifact" }.map(\.args)
    }
    XCTAssertEqual(ids, [["visit-1"], ["visit-2"]])
    await model.stop()
  }
}
