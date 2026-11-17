import AppKit
import SwiftUI
import XCTest

@testable import Releash

final class CreateWorktreeViewTests: IntegrationTestCase {
  private let size = NSSize(
    width: UIStyle.worktreeDialogWidth, height: UIStyle.worktreeDialogHeight)

  @MainActor private func preparedModel(_ http: TestHTTPClient) async throws -> AppModel {
    func repository(_ path: String, _ name: String) -> Releash_Client_V1_WorkspaceRepositoryList {
      .with {
        $0.path = path
        $0.name = name
        $0.branches.items = [
          .with {
            $0.name = "main"
            $0.worktreePath = path
            $0.isMainWorktree = true
          }
        ]
      }
    }
    http.state.withLock { state in
      state.repositories = [repository("/other", "other"), repository("/repo", "releash")]
      state.worktreeBases["/repo"] = .with {
        $0.branches = ["develop", "main", "origin/main"]
        $0.defaultBranch = "main"
      }
      state.availableBranches["/repo"] = .with {
        $0.items = ["feature", "fix/sidebar"].map { name in .with { $0.name = name } }
      }
      state.issues["/repo"] = .with {
        $0.labels = ["bug"]
        $0.milestones = ["v1"]
        $0.issues.items = [
          .with {
            $0.number = 7
            $0.title = "サイドバーを直す"
            $0.defaultBranchName = "feat/issues/7"
          }
        ]
      }
      state.notionTasks["/repo"] = .with {
        $0.page.tasks.items = [
          .with {
            $0.id = "task-1"
            $0.title = "文言を見直す"
            $0.branchName = "feat/task-1"
          }
        ]
      }
      state.notionLabelOptions["/repo"] = .with {
        $0.options = [
          .with {
            $0.propertyName = "Assignee"
            $0.choices = [
              .with {
                $0.label = "Alice"
                $0.value = "user-1"
              }
            ]
          }
        ]
      }
    }
    let model = try await testModel(http)
    model.selectedWorktree = "/repo"
    return model
  }

  private func arguments(_ http: TestHTTPClient, _ target: String) -> [[String]] {
    http.state.withLock { $0.subscriptions.filter { $0.target == target }.map(\.args) }
  }

  @MainActor func testDialogStartsFromSelectedRepositoryAndCreatesCheckedBranch() async throws {
    let http = TestHTTPClient()
    let model = try await preparedModel(http)
    let host = await ViewHarness(CreateWorktreeView(model: model), size: size)
    defer { host.close() }

    try await eventuallyView(host) {
      host.element("repository-picker")?.value == "releash"
        && host.element("base-branch-picker")?.value == "main"
        && host.element("worktree-source-item:branch:feature") != nil
    }
    XCTAssertEqual(host.element("作成")?.enabled, false)
    XCTAssertEqual(host.element("base-branch-picker")?.enabled, false)
    XCTAssertEqual(arguments(http, "available-branches"), [["/repo"]])
    XCTAssertTrue(arguments(http, "notion-label-options").isEmpty)

    try await host.press("worktree-source-item:branch:feature")
    try await host.press("作成")

    try await eventually { http.requests("CreateWorktrees").count == 1 }
    let request = try Releash_Client_V1_CreateWorktreesRequest(
      serializedBytes: XCTUnwrap(http.requests("CreateWorktrees").last?.message))
    XCTAssertEqual(request.repoPath, "/repo")
    XCTAssertEqual(request.branches, ["feature"])
    XCTAssertEqual(request.baseBranch, "main")
    XCTAssertNil(model.failure)
    await model.stop()
  }

  @MainActor func testSwitchingSourceClearsChecksAndSearchReachesServer() async throws {
    let http = TestHTTPClient()
    let model = try await preparedModel(http)
    let host = await ViewHarness(CreateWorktreeView(model: model), size: size)
    defer { host.close() }
    try await host.press("worktree-source-item:branch:feature")
    try await eventuallyView(host) { host.element("作成")?.enabled == true }

    try await host.press("GitHub Issue")
    try await eventuallyView(host) {
      host.element("worktree-source-item:issue:7")?.value == "0"
        && host.element("作成")?.enabled == false
        && host.element("base-branch-picker")?.enabled == true
    }
    XCTAssertNil(host.element("worktree-source-item:branch:feature"))
    XCTAssertEqual(host.element("worktree-source-filter:Label")?.value, "すべて")
    XCTAssertEqual(host.element("worktree-source-filter:Milestone")?.value, "すべて")
    try await host.focusTextField()
    try await host.setTextField("サイドバー")
    try await eventually { self.arguments(http, "issues").last == ["/repo", "search=サイドバー"] }

    try await host.press("Notion task")
    try await eventuallyView(host) {
      host.element("worktree-source-item:task:task-1") != nil
        && host.element("worktree-source-filter:Assignee")?.value == "すべて"
    }
    XCTAssertEqual(arguments(http, "notion-label-options"), [["/repo"]])
    try await eventually {
      self.arguments(http, "notion-tasks").last == ["/repo", "100", "title=サイドバー"]
    }
    await model.stop()
  }

  @MainActor func testRepositoriesArrivingAfterOpenSelectTheDefaultRepository() async throws {
    let http = TestHTTPClient()
    let model = try await preparedModel(http)
    try await eventually { model.repositories.count == 2 }
    let repositories = model.repositories
    model.repositories = []
    let host = await ViewHarness(CreateWorktreeView(model: model), size: size)
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("repository-picker")?.value == "選択してください"
    }

    model.repositories = repositories

    try await eventuallyView(host) {
      host.element("repository-picker")?.value == "releash"
        && host.element("worktree-source-item:branch:feature") != nil
    }
    await model.stop()
  }

  @MainActor func testIssueReadFailureKeepsSelectedFilter() async throws {
    let http = TestHTTPClient()
    let model = try await preparedModel(http)
    var form = CreateWorktreeForm()
    form.repository = "/repo"
    form.source = .issue
    form.label = "bug"
    let host = await ViewHarness(CreateWorktreeView(model: model, form: form), size: size)
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("worktree-source-item:issue:7") != nil
        && host.element("worktree-source-filter:Label")?.value == "bug"
    }

    var payload = Releash_Client_V1_StatePayload()
    payload.issues.readError = "gh: could not resolve host"
    try http.publish(payload, target: "issues")

    try await eventuallyView(host) {
      host.element("worktree-source-failure") != nil
        && host.element("worktree-source-item:issue:7") == nil
    }
    XCTAssertEqual(host.element("worktree-source-filter:Label")?.value, "bug")
    XCTAssertEqual(arguments(http, "issues"), [["/repo", "label=bug"]])
    await model.stop()
  }

  @MainActor func testBaseWithoutDefaultBranchStartsFromHeadAndReadFailureIsShown() async throws {
    let http = TestHTTPClient()
    let model = try await preparedModel(http)
    http.state.withLock { state in
      state.worktreeBases["/repo"] = .with { $0.branches = ["topic"] }
      state.notionTasks["/repo"] = .with { $0.readError.message = "Notion の設定がありません" }
    }
    var form = CreateWorktreeForm()
    form.source = .notion
    let host = await ViewHarness(CreateWorktreeView(model: model, form: form), size: size)
    defer { host.close() }

    try await eventuallyView(host) {
      host.element("base-branch-picker")?.value == "今の HEAD"
        && host.element("worktree-source-failure") != nil
    }
    XCTAssertTrue(host.texts().contains("Notion の設定がありません"))
    await model.stop()
  }
}
