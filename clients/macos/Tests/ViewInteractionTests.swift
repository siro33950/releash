import AppKit
import SwiftProtobuf
import SwiftUI
import XCTest
import ViewInspector

@testable import Bonsplit
@testable import Releash

final class ViewInteractionTests: XCTestCase {
  @MainActor func testExecutionRowsSelectTheirWorktreeAndShowItsSavedTabs() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    for name in ["a", "b"] {
      var branch = Releash_Client_V1_WorkspaceBranch()
      branch.name = name
      branch.worktreePath = "/repo/\(name)"
      repository.branches.items.append(branch)
      var worktree = Releash_Client_V1_WorkspaceWorktreeList()
      worktree.path = branch.worktreePath
      if name == "a" {
        for workflow in [false, true] {
          var execution = Releash_Client_V1_WorktreeExecutionSummary()
          execution.id = workflow ? "workflow-a" : "agent-a"
          execution.title = execution.id
          execution.isWorkflow = workflow
          execution.provider.value = .codex
          worktree.executions.append(execution)
        }
      }
      repository.worktrees.items.append(worktree)
    }
    let tabs = [PaneTab(id: "a-session", kind: .session), PaneTab(id: "a-terminal", kind: .terminal)]
    let layoutA = PaneLayout.pane(id: "a-pane", tabs: tabs, active: tabs[1].id)
    let tabB = PaneTab(id: "b-file", kind: .file)
    let layoutB = PaneLayout.pane(id: "b-pane", tabs: [tabB], active: tabB.id)
    http.state.withLock {
      $0.repositories = [repository]
      $0.layouts["/repo/a", default: .init()].paneLayout = layoutA.message
      $0.layouts["/repo/b", default: .init()].paneLayout = layoutB.message
    }
    let model = try await testModel(http)
    model.selectWorktree("/repo/b")
    try await eventually { model.layoutLoaded && model.layout == layoutB }
    var cards: [String: WorktreeCard] = [:]
    let sidebar = SidebarView(model: model, creating: .constant(false), cardDidAppear: {
      cards[$0.branch.worktreePath] = $0
    })
    let host = TestWindow(HStack {
      sidebar.frame(width: UIStyle.sidebarWidth)
      WorkspacePaneView(model: model)
    })
    defer { cards = [:]; host.window.close() }
    try await eventually { cards["/repo/a"] != nil && model.panes.controller.tabBarAccessory != nil }
    for row in ["agent-a", "workflow-a"] {
      let title = row == "agent-a" ? "agent-a" : "workflow-a · 0 nodes"
      _ = try XCTUnwrap(cards["/repo/a"]).inspect().find(text: title)
      let button = try sidebar.inspect().find(ViewType.Button.self) {
        (try? $0.labelView().view(WorktreeCard.self).actualView().branch.worktreePath) == "/repo/a"
      }
      try button.tap()
      try await eventually { model.selectedWorktree == "/repo/a" && model.layoutLoaded && model.layout == layoutA }
      XCTAssertEqual(try model.panes.layout(), layoutA)
      let central = try host.view.rootView.inspect().find(WorkspacePaneView.self)
      XCTAssertTrue(try central.actualView().model.panes === model.panes)
      model.selectWorktree("/repo/b")
      try await eventually { model.layoutLoaded && model.layout == layoutB }
    }
    await model.stop()
  }

  @MainActor func testIssuePickerInputsSendConditionsAndDisplayServerSnapshots() async throws {
    let http = TestHTTPClient()
    var repository = Releash_Client_V1_WorkspaceRepositoryList()
    repository.path = "/repo"
    let cases: [([String], [String])] = [
      (["/repo"], ["Unfiltered first", "Unfiltered second"]),
      (["/repo", "label=bug"], ["Label result"]),
      (["/repo", "milestone=v1"], ["Milestone result"]),
      (["/repo", "label=bug", "milestone=v1"], ["Both result"]),
    ]
    http.state.withLock { state in
      state.repositories = [repository]
      for (args, titles) in cases {
        var snapshot = Releash_Client_V1_IssuesSnapshot()
        snapshot.labels = ["bug"]
        snapshot.milestones = ["v1"]
        snapshot.issues.items = titles.enumerated().map { index, title in
          var issue = Releash_Client_V1_IssueInfoDto()
          issue.number = UInt64(index + 1)
          issue.title = title
          issue.defaultBranchName = "feat/\(index)"
          return issue
        }
        state.issues[args] = snapshot
      }
    }
    let model = try await testModel(http)
    var shown: CreateWorktreeView?
    let form = CreateWorktreeView(model: model, didAppear: { shown = $0 })
    let host = TestWindow(form, size: NSSize(width: 560, height: 560))
    defer { shown = nil; host.window.close() }
    try await eventually { shown != nil }
    func picker(_ title: String) throws -> InspectableView<ViewType.Picker> {
      try XCTUnwrap(shown).inspect().find(ViewType.Picker.self) {
        try $0.labelView().text().string() == title
      }
    }
    try picker("Repository").select(value: "/repo")
    try picker("作成元").select(value: "issue")
    try await eventually { http.state.withLock { $0.subscriptions.contains { $0.target == "issues" && $0.args == ["/repo"] } } }
    for (label, milestone, index) in [("bug", "", 1), ("", "v1", 2), ("bug", "v1", 3), ("", "", 0)] {
      try picker("Label").select(value: label)
      try picker("Milestone").select(value: milestone)
      let (args, titles) = cases[index]
      try await eventually { http.state.withLock { $0.subscriptions.contains { $0.target == "issues" && $0.args == args } } }
      try await eventually {
        guard let shown else { return false }
        return titles.enumerated().allSatisfy { index, title in
          (try? shown.inspect().find(ViewType.Toggle.self) {
            try $0.labelView().text().string() == "#\(index + 1) \(title)"
          }) != nil
        }
      }
      let toggles = try XCTUnwrap(shown).inspect().findAll(ViewType.Toggle.self)
      XCTAssertEqual(try toggles.map { try $0.labelView().text().string() }, titles.enumerated().map { "#\($0.offset + 1) \($0.element)" })
    }
    await model.stop()
  }

  @MainActor func testPaneChangesSaveThroughTheHostedViewAndRestorePerWorktree() async throws {
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let path = "/repo/a"
    model.selectWorktree(path)
    try await eventually { model.layoutLoaded }
    let host = TestWindow(WorkspacePaneView(model: model))
    defer { host.window.close() }
    try await eventually { model.panes.controller.tabBarAccessory != nil }
    let controller = model.panes.controller
    let first = try XCTUnwrap(controller.allPaneIds.first)
    var expected: PaneLayout = try model.panes.layout()
    var saveCount = http.requests("SaveWorkspaceState").count
    func saved() async throws {
      expected = try model.panes.layout()
      try await eventually {
        http.requests("SaveWorkspaceState").count > saveCount && model.pendingSaves[path] == nil
      }
      let request = try Releash_Client_V1_SaveWorkspaceStateRequest(
        serializedBytes: XCTUnwrap(http.requests("SaveWorkspaceState").last?.message))
      XCTAssertEqual(request.worktreeName, path)
      XCTAssertEqual(try PaneLayout(request.state.paneLayout), expected)
      saveCount = http.requests("SaveWorkspaceState").count
      try await Task.sleep(for: .milliseconds(50))
    }
    for kind in PaneTab.Kind.allCases {
      model.panes.open(kind, in: first)
      try await saved()
    }
    let second = try XCTUnwrap(controller.splitPane(first, orientation: .horizontal))
    try await saved()
    let terminal = try XCTUnwrap(controller.internalController.rootNode.findPane(first)?.tabs.first)
    controller.internalController.moveTab(terminal, from: first, to: second)
    try await saved()
    model.panes.open(.file, in: second)
    try await saved()
    let third = try XCTUnwrap(controller.splitPane(second, orientation: .vertical))
    try await saved()
    model.panes.open(.workflow, in: third)
    try await saved()
    let outer = controller.treeSnapshot()
    guard case .split(let outerSplit) = outer, case .split(let innerSplit) = outerSplit.second else {
      return XCTFail("Missing nested splits")
    }
    XCTAssertTrue(controller.setDividerPosition(0.3, forSplit: try XCTUnwrap(UUID(uuidString: outerSplit.id))))
    try await saved()
    XCTAssertTrue(controller.setDividerPosition(0.65, forSplit: try XCTUnwrap(UUID(uuidString: innerSplit.id))))
    try await saved()
    let originalTabs = controller.tabs(inPane: first)
    let last = try XCTUnwrap(controller.internalController.rootNode.findPane(first)?.tabs.last)
    controller.internalController.moveTab(last, from: first, to: first, atIndex: 0)
    try await saved()
    XCTAssertEqual(controller.tabs(inPane: first).map(\.id), [originalTabs[2].id, originalTabs[0].id, originalTabs[1].id])
    controller.selectTab(originalTabs[0].id)
    try await saved()
    XCTAssertTrue(controller.closeTab(originalTabs[1].id))
    try await saved()
    guard case .split(_, let axis, let ratio, let left, let right) = expected,
      case .split(_, let nestedAxis, let nestedRatio, let top, let bottom) = right,
      case .pane(_, let leftTabs, let selected) = left
    else { return XCTFail("Missing saved nested panes") }
    XCTAssertEqual(axis, .horizontal)
    XCTAssertEqual(ratio, 0.3)
    XCTAssertEqual(nestedAxis, .vertical)
    XCTAssertEqual(nestedRatio, 0.65)
    XCTAssertEqual(leftTabs.map(\.kind), [.session, .workflow])
    XCTAssertEqual(selected, originalTabs[0].id.id.uuidString)
    XCTAssertEqual(top.tabs.map(\.kind), [.terminal, .file])
    XCTAssertEqual(bottom.tabs.map(\.kind), [.workflow])
    model.selectWorktree("/repo/b")
    try await eventually { model.layoutLoaded && model.layout.tabs.isEmpty }
    model.selectWorktree(path)
    try await eventually { model.layoutLoaded && model.layout == expected }
    XCTAssertEqual(try model.panes.layout(), expected)
    await model.stop()
    host.window.close()
    let restarted = try await testModel(http)
    restarted.selectWorktree(path)
    try await eventually { restarted.layoutLoaded }
    let restoredHost = TestWindow(WorkspacePaneView(model: restarted))
    defer { restoredHost.window.close() }
    try await eventually { restarted.panes.controller.tabBarAccessory != nil }
    XCTAssertEqual(restarted.layout, expected)
    XCTAssertEqual(try restarted.panes.layout(), expected)
    await restarted.stop()
  }

  @MainActor func testRightPanelSelectionAndCollapseKeepTheLastPanel() async throws {
    var shown: RightPanelView?
    let panel = RightPanelTestView(didAppear: { shown = $0 })
    let host = TestWindow(panel, size: NSSize(width: 280, height: 600))
    defer { shown = nil; host.window.close() }
    try await eventually { shown != nil }
    func inspected() throws -> InspectableView<ViewType.ClassifiedView> {
      try XCTUnwrap(shown).inspect()
    }
    for title in ["ファイル", "Source Control", "Thread", "タスク"] {
      try inspected().find(viewWithAccessibilityLabel: title).button().tap()
      let heading = try inspected().find(viewWithAccessibilityIdentifier: "right-panel-title").text()
      XCTAssertEqual(try heading.string(), title)
    }
    try inspected().find(viewWithAccessibilityLabel: "右パネルを閉じる").button().tap()
    XCTAssertFalse(try XCTUnwrap(shown).expanded)
    XCTAssertThrowsError(try inspected().find(viewWithAccessibilityIdentifier: "right-panel-title"))
    XCTAssertEqual(try inspected().findAll(ViewType.Button.self).count, 1)
    let frame = try inspected().implicitAnyView().vStack().flexFrame()
    XCTAssertEqual(frame.minWidth, UIStyle.collapsedPanelWidth)
    XCTAssertEqual(frame.idealWidth, UIStyle.collapsedPanelWidth)
    XCTAssertEqual(frame.maxWidth, UIStyle.collapsedPanelWidth)
    try inspected().find(viewWithAccessibilityLabel: "右パネルを開く").button().tap()
    XCTAssertTrue(try XCTUnwrap(shown).expanded)
    XCTAssertEqual(try inspected().findAll(ViewType.Button.self).count, 5)
    XCTAssertEqual(try inspected().find(viewWithAccessibilityIdentifier: "right-panel-title").text().string(), "タスク")
  }
}

private struct RightPanelTestView: View {
  @State private var expanded = true
  let didAppear: (RightPanelView) -> Void
  var body: some View {
    RightPanelView(expanded: $expanded, didAppear: didAppear)
  }
}
