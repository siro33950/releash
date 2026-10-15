import XCTest

@testable import Releash

final class E2EFixtureTests: XCTestCase {
  func testCleanupAttemptsBothOperationsAndPreservesFirstFailure() throws {
    for stopFails in [false, true] {
      for removeFails in [false, true] {
        var calls: [String] = []
        do {
          try Fixture.cleanUp(
            stop: {
              calls.append("stop")
              if stopFails { throw NSError(domain: "stop", code: 1) }
            },
            remove: {
              calls.append("remove")
              if removeFails { throw NSError(domain: "remove", code: 2) }
            })
          XCTAssertFalse(stopFails || removeFails)
        } catch {
          XCTAssertEqual((error as NSError).domain, stopFails ? "stop" : "remove")
        }
        XCTAssertEqual(calls, ["stop", "remove"])
      }
    }
  }

  func testProvidersMustBothResolveToConfiguredFixture() throws {
    var availability = Releash_Client_V1_ProviderAvailabilitySnapshotResponse()
    for provider in ["claude", "codex"] {
      var item = Releash_Client_V1_ProviderAvailabilityItemResponse()
      item.provider = provider
      item.resolvedExecutable = "/fixture"
      availability.providers.items.append(item)
    }
    XCTAssertNoThrow(try Fixture.verifyProviders(availability, executable: "/fixture"))
    for index in availability.providers.items.indices {
      var wrong = availability
      wrong.providers.items[index].resolvedExecutable = "/real-provider"
      XCTAssertThrowsError(try Fixture.verifyProviders(wrong, executable: "/fixture"))
      wrong.providers.items[index].clearResolvedExecutable()
      XCTAssertThrowsError(try Fixture.verifyProviders(wrong, executable: "/fixture"))
      wrong.providers.items.remove(at: index)
      XCTAssertThrowsError(try Fixture.verifyProviders(wrong, executable: "/fixture"))
    }
  }

  func testWorkflowMustBeActiveInRequestedRepositoryAndWorktree() {
    var execution = Releash_Client_V1_WorktreeExecutionSummary()
    execution.id = "execution"
    execution.isWorkflow = true
    execution.status = "active"
    var worktree = Releash_Client_V1_WorkspaceWorktreeList()
    worktree.path = "/canonical/repo"
    worktree.executions = [execution]
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    var main = Releash_Client_V1_WorkspaceBranch()
    main.isMainWorktree = true
    main.worktreePath = "/canonical/repo"
    repository.branches.items = [main]
    repository.worktrees.items = [worktree]
    var snapshot = Releash_Client_V1_WorkspaceListSnapshot()
    snapshot.repositories.items = [repository]
    XCTAssertTrue(
      Fixture.hasRunningWorkflow(snapshot, repository: "/repo", executionID: "execution"))
    var withoutMain = snapshot
    withoutMain.repositories.items[0].branches.items[0].isMainWorktree = false
    XCTAssertFalse(
      Fixture.hasRunningWorkflow(withoutMain, repository: "/repo", executionID: "execution"))
    withoutMain.repositories.items[0].branches.items.removeAll()
    XCTAssertFalse(
      Fixture.hasRunningWorkflow(withoutMain, repository: "/repo", executionID: "execution"))
    XCTAssertFalse(
      Fixture.hasRunningWorkflow(snapshot, repository: "/other", executionID: "execution"))
    XCTAssertFalse(Fixture.hasRunningWorkflow(snapshot, repository: "/repo", executionID: "other"))
    for status in ["attention", "idle", ""] {
      snapshot.repositories.items[0].worktrees.items[0].executions[0].status = status
      XCTAssertFalse(
        Fixture.hasRunningWorkflow(snapshot, repository: "/repo", executionID: "execution"))
    }
    snapshot.repositories.items[0].worktrees.items[0].executions[0].status = "active"
    snapshot.repositories.items[0].worktrees.items[0].executions[0].isWorkflow = false
    XCTAssertFalse(
      Fixture.hasRunningWorkflow(snapshot, repository: "/repo", executionID: "execution"))
    snapshot.repositories.items[0].worktrees.items[0].path = "/other"
    XCTAssertFalse(
      Fixture.hasRunningWorkflow(snapshot, repository: "/repo", executionID: "execution"))
  }
}
