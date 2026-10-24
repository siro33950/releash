import AppKit
import SwiftUI
import XCTest

@testable import Releash

final class SidebarViewTests: XCTestCase {
  @MainActor func testCreatedWorkflowsKeepSidebarAndFooterUpdatingWithMainWorktrees() async throws {
    continueAfterFailure = false
    let http = TestHTTPClient()
    var repositories = (0..<3).map { index in
      var repository = Releash_Client_V1_WorkspaceRepositoryList()
      repository.path = "/repo-\(index)"
      repository.branches.items = ["main", "b", "m", "z"].map { name in
        var branch = Releash_Client_V1_WorkspaceBranch()
        branch.name = name
        branch.worktreePath =
          name == "main" ? repository.path : "/repo-\(index)-worktrees/\(name)"
        branch.isMainWorktree = name == "main"
        return branch
      }
      return repository
    }
    http.state.withLock { $0.repositories = repositories }
    let model = try await testModel(http)
    let host = await ViewHarness(RootView(model: model), size: NSSize(width: 1100, height: 1200))
    defer { host.close() }
    func outlines(_ view: NSView) -> [NSOutlineView] {
      (view as? NSOutlineView).map { [$0] } ?? view.subviews.flatMap(outlines)
    }
    let content = try XCTUnwrap(host.window.contentView)
    try await eventuallyView(host) { outlines(content).first?.numberOfRows == 15 }
    let outline = try XCTUnwrap(outlines(content).first)
    let parent = try XCTUnwrap(outline.item(atRow: 0) as? NSObject)

    for source in ["issues", "tasks"] {
      var form = CreateWorktreeForm()
      form.repository = repositories[0].path
      form.launch = "workflow"
      form.workflow = "review"
      for index in 1...2 {
        let selection: WorktreeSelection.Key =
          source == "issues" ? .issue(UInt64(index)) : .task("task-\(index)")
        form.selected.set(selection, branch: "feat/\(source)/\(index)", selected: true)
      }
      form.submit(model: model, dismiss: {})
      try await eventually {
        http.requests("CreateWorktrees").count == (source == "issues" ? 1 : 2)
      }
      let request = try Releash_Client_V1_CreateWorktreesRequest(
        serializedBytes: XCTUnwrap(http.requests("CreateWorktrees").last?.message))
      guard case .workflow(let launch) = request.launch else {
        return XCTFail("Missing workflow launch")
      }
      XCTAssertEqual(launch.name, "review")
      XCTAssertEqual(request.repoPath, repositories[0].path)
      XCTAssertEqual(request.branches.count, 2)

      for name in request.branches {
        var branch = Releash_Client_V1_WorkspaceBranch()
        branch.name = name
        branch.worktreePath = "/repo-0-worktrees/\(name)"
        repositories[0].branches.items.insert(branch, at: 1)
        var payload = Releash_Client_V1_StatePayload()
        payload.workspaces.repositories.items = repositories
        try http.publish(payload, target: "workspaces")
        let expectedRows = 3 + repositories.reduce(0) { $0 + $1.branches.items.count }
        try await eventuallyView(host) {
          outline.numberOfRows == expectedRows && host.element("worktree:\(name)") != nil
        }
        XCTAssertEqual(
          outline.parent(forItem: try XCTUnwrap(outline.item(atRow: 2))) as? NSObject, parent)

        var execution = Releash_Client_V1_WorktreeExecutionSummary()
        execution.id = branch.worktreePath + "/workflow"
        execution.title = "Workflow \(name)"
        execution.isWorkflow = true
        execution.status = "running"
        execution.nodeCount = 3
        execution.sessionStates = ["running", "pending", "pending"]
        var worktree = Releash_Client_V1_WorkspaceWorktreeList()
        worktree.path = branch.worktreePath
        worktree.executions = [execution]
        repositories[0].worktrees.items.append(worktree)
        payload.workspaces.repositories.items = repositories
        try http.publish(payload, target: "workspaces")
        try await eventuallyView(host) {
          host.element("worktree:\(name)")?.label.contains("Workflow \(name) · 3 nodes") == true
        }

        var info = Releash_Client_V1_StatePayload()
        info.daemonInfo.release = "after-\(name)"
        try http.publish(info, target: "daemon-info")
        try await eventuallyView(host) {
          host.element("footer-server")?.label.contains("after-\(name)") == true
        }
      }
    }

    repositories[0].worktrees.items[0].executions[0].title = "Completed workflow"
    repositories[0].worktrees.items[0].executions[0].status = "completed"
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = repositories
    try http.publish(payload, target: "workspaces")
    try await eventuallyView(host) {
      host.element("worktree:feat/issues/1")?.label.contains("Completed workflow · 3 nodes") == true
    }
    for tick in 1...2 {
      var info = Releash_Client_V1_StatePayload()
      info.daemonInfo.release = "footer-only-\(tick)"
      try http.publish(info, target: "daemon-info")
      try await eventuallyView(host) {
        host.element("footer-server")?.label.contains("footer-only-\(tick)") == true
      }
    }
    XCTAssertNil(model.failure)
    await model.stop()
  }
}
