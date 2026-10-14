import Connect
import SwiftProtobuf
import SwiftUI
import XCTest

@testable import Releash

final class WorkspaceActionsTests: XCTestCase {
  @MainActor func testLaunchFailureDisplaysServerReasonWithoutOpeningTabs() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var detail = Releash_Client_V1_CommandError()
    detail.message.value = "開始できない理由"
    let packed = ConnectError.Detail(type: Releash_Client_V1_CommandError.protoMessageName, payload: try detail.serializedData())
    for method in ["CreateAgentSession", "StartWorkflow"] {
      for serverDetail in [nil, packed] {
        http.state.withLock { $0.failedMethod = method; $0.failureDetail = serverDetail }
        do {
          if method == "CreateAgentSession" { try await model.startSession(provider: "codex", path: "/repo/a") }
          else { try await model.startWorkflow(name: "review", path: "/repo/a") }
          XCTFail("Expected launch failure")
        } catch {
          XCTAssertEqual(model.failureMessage(error), serverDetail == nil ? "\(method) failed" : "開始できない理由")
        }
        XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
      }
    }
    await model.stop()
  }

  @MainActor func testFailedLaunchDoesNotOpenTabs() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    for method in ["CreateAgentSession", "StartWorkflow"] {
      http.state.withLock { $0.failedMethod = method }
      do {
        if method == "CreateAgentSession" {
          try await model.startSession(provider: "codex", path: "/repo/a")
        } else {
          try await model.startWorkflow(name: "review", path: "/repo/a")
        }
        XCTFail("Expected launch failure")
      } catch {
        XCTAssertTrue(model.panes.controller.allTabIds.isEmpty)
      }
    }
    await model.stop()
  }

  @MainActor func testRepositoryActionsAndReadAndOperationFailures() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    repository.status.error = "repository read failed"
    http.state.withLock { $0.repositories = [repository] }
    let model = try await testModel(http)
    try await eventually { model.repositories.count == 1 }
    let sidebar = SidebarView(model: model, creating: .constant(false))
    XCTAssertEqual(sidebar.repositoryFailure(model.repositories[0]), "repository read failed")
    XCTAssertNil(sidebar.repositoryFailure(.init()))
    model.addRepository("/added/repo")
    model.refresh()
    try await eventually {
      !http.requests("AddRepoPath").isEmpty && !http.requests("RefreshWorkspaces").isEmpty
    }
    XCTAssertEqual(
      try Releash_Client_V1_AddRepoPathRequest(
        serializedBytes: XCTUnwrap(http.requests("AddRepoPath").first?.message)
      ).path, "/added/repo")
    http.state.withLock { $0.failedMethod = "RefreshWorkspaces" }
    model.refresh()
    try await eventually { model.failure?.contains("RefreshWorkspaces failed") == true }
    await model.stop()
  }

  @MainActor func testCardReceivesGitMergedAndCompletedPRWithoutOpenPR() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    var merged = Releash_Client_V1_WorkspaceBranch()
    merged.name = "merged-in-git"
    merged.worktreePath = "/repo/merged"
    merged.isMerged = true
    merged.hasPr_p = false
    var completed = Releash_Client_V1_WorkspaceBranch()
    completed.name = "completed-pr"
    completed.worktreePath = "/repo/completed"
    completed.hasPr_p = false
    completed.prState = "closed"
    completed.prStateNumber = 42
    repository.branches.items = [merged, completed]
    http.state.withLock { $0.repositories = [repository] }
    let model = try await testModel(http)
    try await eventually { model.repositories.first?.branches.items.count == 2 }
    let received = try XCTUnwrap(model.repositories.first)
    let mergedCard = WorktreeCard(
      model: model, repository: received, branch: received.branches.items[0])
    XCTAssertTrue(mergedCard.branch.isMerged)
    XCTAssertFalse(mergedCard.branch.hasPrState)
    let completedCard = WorktreeCard(
      model: model, repository: received, branch: received.branches.items[1])
    XCTAssertEqual(completedCard.branch.prState, "closed")
    XCTAssertEqual(completedCard.branch.prStateNumber, 42)
    XCTAssertFalse(completedCard.branch.hasPr_p)
    XCTAssertFalse(completedCard.branch.hasPrNumber)
    XCTAssertFalse(completedCard.branch.hasPrURL)
    await model.stop()
  }

  @MainActor func testCardLaunchesDeletionConfirmationAndRowsUseWorktree() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    var branch = Releash_Client_V1_WorkspaceBranch()
    branch.worktreePath = "/repo/a"
    for force in [false, true] {
      branch.removalRequiresForce = force
      let card = WorktreeCard(model: model, repository: repository, branch: branch)
      XCTAssertTrue(card.canRemove)
      XCTAssertEqual(card.removalButtonTitle, force ? "強制削除" : "削除")
      XCTAssertEqual(
        card.removalTitle, force ? "変更またはロックがあるWorktreeを強制削除しますか？" : "Worktreeを削除しますか？")
      card.remove()
      try await eventually { http.requests("RemoveWorktree").count == (force ? 2 : 1) }
      let removal = try Releash_Client_V1_RemoveWorktreeRequest(
        serializedBytes: XCTUnwrap(http.requests("RemoveWorktree").last?.message))
      XCTAssertEqual(removal.repoPath, "/repo")
      XCTAssertEqual(removal.worktreePath, "/repo/a")
      XCTAssertEqual(removal.force, force)
    }
    branch.removalRequiresForce = false
    branch.isMainWorktree = true
    XCTAssertFalse(WorktreeCard(model: model, repository: repository, branch: branch).canRemove)
    branch.isMainWorktree = false
    branch.isDeleting = true
    XCTAssertFalse(WorktreeCard(model: model, repository: repository, branch: branch).canRemove)
    branch.isDeleting = false
    for isWorkflow in [false, true] {
      var execution = Releash_Client_V1_WorktreeExecutionSummary()
      execution.isWorkflow = isWorkflow
      var worktree = Releash_Client_V1_WorkspaceWorktreeList()
      worktree.path = branch.worktreePath
      worktree.executions = [execution]
      repository.worktrees.items = [worktree]
      let card = WorktreeCard(model: model, repository: repository, branch: branch)
      XCTAssertEqual(card.worktree?.executions.first?.isWorkflow, isWorkflow)
      model.selectWorktree("/repo/b")
      model.selectWorktree(branch.worktreePath)
      XCTAssertEqual(model.selectedWorktree, "/repo/a")
    }
    var provider = Releash_Client_V1_AgentSessionProviderDto()
    provider.value = .codex
    try await eventually { model.layoutLoaded }
    try await model.startSession(provider: provider.wireName, path: branch.worktreePath)
    var root = Releash_Client_V1_WorkspaceTreeItem()
    root.sequence.id = "workflow-execution"
    root.sequence.title = "review"
    var tree = Releash_Client_V1_WorkspaceWorktreeList()
    tree.path = branch.worktreePath; tree.snapshot.nodes.items = [root]
    repository.worktrees.items = [tree]; model.repositories = [repository]
    try await model.startWorkflow(name: "review", path: branch.worktreePath)
    XCTAssertEqual(try model.panes.layout().tabs.map(\.kind), [.session, .workflow])
    model.panes.closeCurrentTab()
    model.panes.closeCurrentTab()
    XCTAssertTrue(http.requests("DeleteAgentSession").isEmpty)
    XCTAssertTrue(http.requests("AbortWorkflow").isEmpty)
    try await eventually {
      !http.requests("CreateAgentSession").isEmpty && !http.requests("StartWorkflow").isEmpty
    }
    let session = try Releash_Client_V1_CreateAgentSessionRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateAgentSession").last?.message))
    XCTAssertEqual(session.provider, "codex")
    XCTAssertEqual(session.workspaceIdentity, "/repo/a")
    XCTAssertEqual(session.worktreePath, "/repo/a")
    XCTAssertEqual(session.rows, UIStyle.terminalRows)
    XCTAssertEqual(session.cols, UIStyle.terminalColumns)
    XCTAssertFalse(session.callerRequestID.isEmpty)
    let workflow = try Releash_Client_V1_StartWorkflowRequest(
      serializedBytes: XCTUnwrap(http.requests("StartWorkflow").last?.message))
    XCTAssertEqual(workflow.workflowName, "review")
    XCTAssertEqual(workflow.worktreePath, "/repo/a")
    XCTAssertFalse(workflow.hasRequest)
    await model.stop()
  }

  @MainActor func testUnknownRemovalRequirementUsesNormalConfirmationAndPreservesWorktreeOnFailure() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    var branch = Releash_Client_V1_WorkspaceBranch()
    branch.worktreePath = "/repo/a"
    branch.dirtyCountError = "status unavailable"
    repository.branches.items = [branch]
    var detail = Releash_Client_V1_CommandError()
    detail.message.value = "worktree has 1 uncommitted change(s). Use force to remove."
    let packed = ConnectError.Detail(
      type: Releash_Client_V1_CommandError.protoMessageName, payload: try detail.serializedData())
    http.state.withLock {
      $0.repositories = [repository]
      $0.failedMethod = "RemoveWorktree"
      $0.failureDetail = packed
    }
    let model = try await testModel(http)
    try await eventually { model.repositories.first?.branches.items.count == 1 }
    model.selectWorktree(branch.worktreePath)
    try await eventually { model.layoutLoaded }
    let received = try XCTUnwrap(model.repositories.first)
    let card = WorktreeCard(model: model, repository: received, branch: received.branches.items[0])
    XCTAssertFalse(card.branch.hasRemovalRequiresForce)
    XCTAssertTrue(card.canRemove)
    XCTAssertEqual(card.removalTitle, "Worktreeを削除しますか？")
    XCTAssertEqual(card.removalButtonTitle, "削除")

    card.remove()

    try await eventually { model.failure == detail.message.value }
    let removal = try Releash_Client_V1_RemoveWorktreeRequest(
      serializedBytes: XCTUnwrap(http.requests("RemoveWorktree").last?.message))
    XCTAssertEqual(removal.repoPath, repository.path)
    XCTAssertEqual(removal.worktreePath, branch.worktreePath)
    XCTAssertFalse(removal.force)
    XCTAssertEqual(model.repositories.first?.branches.items.map(\.worktreePath), [branch.worktreePath])
    XCTAssertEqual(model.selectedWorktree, branch.worktreePath)
    await model.stop()
  }

  @MainActor func testCreationInputClosureAndFailureUseSelectedBranchesAndLaunch() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    var form = CreateWorktreeForm()
    var dismissed = false
    XCTAssertFalse(form.canSubmit)
    form.repository = "/repo"
    form.branch = "  "
    form.submit(model: model) { dismissed = true }
    XCTAssertFalse(form.canSubmit)
    XCTAssertFalse(dismissed)
    XCTAssertTrue(http.requests("CreateWorktrees").isEmpty)
    form.selected.set(.issue(1204), branch: "feat/issues/1204", selected: true)
    form.selected.set(.branch("1204"), branch: "1204", selected: true)
    form.baseBranch = "main"
    form.launch = "workflow"
    XCTAssertFalse(form.canSubmit)
    form.workflow = "review"
    form.requestText = "check changes"
    XCTAssertTrue(form.canSubmit)
    http.state.withLock { $0.deferCreation = true }
    form.submit(model: model) { dismissed = true }
    XCTAssertTrue(dismissed)
    XCTAssertNil(model.failure)
    try await eventually { http.state.withLock { $0.pendingCreation != nil } }
    let request = try Releash_Client_V1_CreateWorktreesRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateWorktrees").last?.message))
    XCTAssertEqual(request.repoPath, "/repo")
    XCTAssertEqual(request.baseBranch, "main")
    XCTAssertEqual(request.branches, ["1204", "feat/issues/1204"])
    guard case .workflow(let workflow) = request.launch else { return XCTFail("Missing workflow") }
    XCTAssertEqual(workflow.name, "review")
    XCTAssertFalse(workflow.hasRequest)
    let completion = http.state.withLock { state in
      let completion = state.pendingCreation
      state.pendingCreation = nil
      state.deferCreation = false
      return completion
    }
    completion?(TestHTTPClient.failure("creation or launch failed"))
    try await eventually { model.failure?.contains("creation or launch failed") == true }
    form.launch = "session"
    form.provider = "codex"
    form.submit(model: model) {}
    try await eventually { http.requests("CreateWorktrees").count == 2 }
    let sessionRequest = try Releash_Client_V1_CreateWorktreesRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateWorktrees").last?.message))
    guard case .session(let session) = sessionRequest.launch else {
      return XCTFail("Missing session")
    }
    XCTAssertEqual(session.provider, "codex")
    XCTAssertEqual(session.rows, UIStyle.terminalRows)
    XCTAssertEqual(session.cols, UIStyle.terminalColumns)
    XCTAssertFalse(session.requestID.isEmpty)
    await model.stop()
  }
}
