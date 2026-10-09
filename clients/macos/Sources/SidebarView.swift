import SwiftUI

struct SidebarView: View {
  @Bindable var model: AppModel
  @Binding var creating: Bool
  func repositoryFailure(_ repository: Releash_Client_V1_WorkspaceRepositoryList) -> String? {
    repository.status.hasError ? repository.status.error : nil
  }
  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      HStack {
        Button(action: model.addRepository) {
          Label("Repositoryを追加", systemImage: "folder.badge.plus")
        }.labelStyle(.iconOnly)
        Button(action: model.refresh) { Label("再読み込み", systemImage: "arrow.clockwise") }.labelStyle(
          .iconOnly)
        Spacer()
        Button {
          creating = true
        } label: {
          Label("Worktreeを作成", systemImage: "plus")
        }.labelStyle(.iconOnly)
      }
      ScrollView {
        ForEach(model.repositories, id: \.path) { repository in
          DisclosureGroup(
            isExpanded: Binding(
              get: { !(model.collapsed[repository.path] ?? false) },
              set: { model.setCollapsed(repository.path, !$0) })
          ) {
            if let error = repositoryFailure(repository) {
              Text(error).foregroundStyle(.secondary)
            }
            ForEach(repository.branches.items, id: \.worktreePath) { branch in
              WorktreeCard(model: model, repository: repository, branch: branch)
            }
          } label: {
            Text(URL(fileURLWithPath: repository.path).lastPathComponent)
          }
        }
      }
    }.padding(UIStyle.padding)
  }
}

struct WorktreeCard: View {
  @Bindable var model: AppModel
  let repository: Releash_Client_V1_WorkspaceRepositoryList
  let branch: Releash_Client_V1_WorkspaceBranch
  @State private var deleting = false
  @State private var launchingWorkflow = false
  var worktree: Releash_Client_V1_WorkspaceWorktreeList? {
    repository.worktrees.items.first { $0.path == branch.worktreePath }
  }
  var removalTitle: String {
    branch.removalRequiresForce ? "変更またはロックがあるWorktreeを強制削除しますか？" : "Worktreeを削除しますか？"
  }
  var removalButtonTitle: String { branch.removalRequiresForce ? "強制削除" : "削除" }
  var canRemove: Bool {
    !branch.isMainWorktree && !branch.isDeleting && branch.hasRemovalRequiresForce
  }
  func select() { model.selectWorktree(branch.worktreePath) }
  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      Button {
        select()
      } label: {
        HStack {
          if let worktree, worktree.hasAggregateStatus { status(worktree.aggregateStatus) }
          if branch.isMainWorktree { Image(systemName: "house") }
          Text(branch.name).fontWeight(.medium)
          if branch.isMerged {
            Image(systemName: "checkmark.circle").foregroundStyle(.secondary)
              .accessibilityLabel("Merged").help("Merged")
          }
          Spacer()
          if branch.isDeleting {
            ProgressView().controlSize(.small)
            Text("削除中")
          }
        }
      }.buttonStyle(.plain)
      if branch.hasUpstream || branch.hasPrState {
        HStack {
          if branch.hasUpstream {
            Label("\(branch.ahead)", systemImage: "arrow.up")
            Label("\(branch.behind)", systemImage: "arrow.down")
          }
          if branch.hasPrState {
            Image(systemName: prIcon(branch.prState)).help(
              "PR #\(branch.prStateNumber): \(branch.prState)")
          }
        }.foregroundStyle(.secondary)
      }
      if branch.hasDirtyCount { Text("\(branch.dirtyCount) changes").foregroundStyle(.secondary) }
      if branch.hasDirtyCountError { Text("Changes unavailable").help(branch.dirtyCountError) }
      if branch.hasPullRequestError { Text("PR unavailable").help(branch.pullRequestError) }
      if let worktree {
        if worktree.status.hasError { Text(worktree.status.error).foregroundStyle(.secondary) }
        ForEach(worktree.executions, id: \.id) { execution in
          Button {
            select()
          } label: {
            HStack(spacing: UIStyle.spacing) {
              if execution.isWorkflow {
                Text("\(execution.title) · \(execution.nodeCount) nodes")
                ForEach(Array(execution.sessionStates.enumerated()), id: \.offset) { _, state in
                  Rectangle().fill(UIStyle.statusColor(state)).frame(
                    width: UIStyle.statusSize, height: UIStyle.statusSize)
                }
              } else {
                status(execution.status)
                Image(systemName: execution.provider == "claude" ? "sparkle" : "terminal")
                Text(execution.title)
              }
            }
          }.buttonStyle(.plain)
        }
      }
    }
    .padding(UIStyle.padding)
    .background(
      model.selectedWorktree == branch.worktreePath
        ? Color.accentColor.opacity(0.12) : Color(nsColor: .controlBackgroundColor),
      in: UIStyle.cardShape
    )
    .contextMenu {
      Menu("Sessionを追加") {
        ForEach(model.providers, id: \.value) { provider in
          Button(provider.displayName) { addSession(provider) }
        }
      }
      Button("Workflowを追加") { launchingWorkflow = true }
      Button("削除", role: .destructive) { deleting = true }
        .disabled(!canRemove)
    }
    .confirmationDialog(
      removalTitle,
      isPresented: $deleting
    ) {
      Button(removalButtonTitle, role: .destructive) { remove() }
    }
    .sheet(isPresented: $launchingWorkflow) {
      WorkflowLaunchView(model: model, path: branch.worktreePath)
    }
  }
  private func status(_ value: String) -> some View {
    Circle().fill(UIStyle.statusColor(value)).frame(
      width: UIStyle.statusSize, height: UIStyle.statusSize
    ).accessibilityLabel(value)
  }
  private func prIcon(_ value: String) -> String {
    switch value {
    case "open": "arrow.triangle.pull"
    case "draft": "ellipsis.circle"
    case "merged": "arrow.triangle.merge"
    default: "xmark.circle"
    }
  }
  func addSession(_ provider: Releash_Client_V1_AgentSessionProviderDto) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_CreateAgentSessionRequest()
    request.provider = provider.wireName
    request.workspaceIdentity = branch.worktreePath
    request.worktreePath = branch.worktreePath
    request.rows = UIStyle.terminalRows
    request.cols = UIStyle.terminalColumns
    request.callerRequestID = UUID().uuidString
    model.perform {
      _ = try await client.createAgentSession(request: request, headers: model.headers).result.get()
    }
  }
  func remove() {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_RemoveWorktreeRequest()
    request.repoPath = repository.path
    request.worktreePath = branch.worktreePath
    request.force = branch.removalRequiresForce
    model.perform {
      _ = try await client.removeWorktree(request: request, headers: model.headers).result.get()
    }
  }
}

struct WorkflowLaunchView: View {
  @Bindable var model: AppModel
  let path: String
  @Environment(\.dismiss) private var dismiss
  @State private var workflow = ""
  @State private var requestText = ""
  var body: some View {
    Form {
      WorkflowLaunchFields(
        workflows: model.workflows, workflow: $workflow, requestText: $requestText)
      HStack {
        Button("キャンセル") { dismiss() }
        Button("起動") {
          launch(name: workflow, requestText: requestText)
          dismiss()
        }.disabled(workflow.isEmpty)
      }
    }.padding(UIStyle.padding)
  }
  func launch(name: String, requestText: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_StartWorkflowRequest()
    request.workflowName = name
    request.worktreePath = path
    request.request = requestText
    model.perform {
      _ = try await client.startWorkflow(request: request, headers: model.headers).result.get()
    }
  }
}
