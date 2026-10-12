import SwiftUI

struct SidebarView: View {
  @Bindable var model: AppModel
  @Binding var creating: Bool
  func repositoryFailure(_ repository: Releash_Client_V1_WorkspaceRepositoryList) -> String? {
    repository.status.hasError ? repository.status.error : nil
  }
  var body: some View {
    VStack(spacing: 0) {
      Color.clear.frame(height: UIStyle.tabBarHeight)
      Divider()
      HStack {
        Text("Workspaces").font(.callout).foregroundStyle(.secondary)
        Spacer()
        Button(action: model.addRepository) {
          Image(systemName: "folder.badge.plus")
        }.help("Repositoryを追加").accessibilityLabel("Repositoryを追加")
        Button(action: model.refresh) {
          Image(systemName: "arrow.clockwise")
        }.help("再読み込み").accessibilityLabel("再読み込み")
        Button {
          creating = true
        } label: {
          Image(systemName: "plus")
        }.help("Worktreeを作成").accessibilityLabel("Worktreeを作成")
      }
      .buttonStyle(.borderless)
      .padding(UIStyle.padding)
      List {
        ForEach(model.repositories, id: \.path) { repository in
          Section(
            isExpanded: Binding(
              get: { !(model.collapsed[repository.path] ?? false) },
              set: { model.setCollapsed(repository.path, !$0) }
            )
          ) {
            if let error = repositoryFailure(repository) {
              Text(error).foregroundStyle(.secondary)
            }
            ForEach(repository.branches.items, id: \.worktreePath) { branch in
              Button {
                model.selectWorktree(branch.worktreePath)
              } label: {
                WorktreeCard(model: model, repository: repository, branch: branch)
                  .frame(maxWidth: .infinity, alignment: .leading)
                  .padding(.horizontal, UIStyle.rowSpacing)
                  .contentShape(Rectangle())
                  .background(
                    model.selectedWorktree == branch.worktreePath
                      ? UIStyle.sidebarSelection : .clear,
                    in: RoundedRectangle(cornerRadius: UIStyle.spacing))
              }
              .buttonStyle(.plain)
              .foregroundStyle(.primary)
              .accessibilityAddTraits(
                model.selectedWorktree == branch.worktreePath ? .isSelected : [])
            }
          } header: {
            HStack(spacing: UIStyle.rowSpacing) {
              Text(URL(fileURLWithPath: repository.path).lastPathComponent)
              Text(repository.branches.items.count, format: .number)
                .foregroundStyle(.secondary)
            }
            .selectionDisabled()
          }
        }
      }
      .listStyle(.sidebar)
      .scrollContentBackground(.hidden)
    }
  }

}

struct WorktreeCard: View {
  @Bindable var model: AppModel
  let repository: Releash_Client_V1_WorkspaceRepositoryList
  let branch: Releash_Client_V1_WorkspaceBranch
  @State private var deleting = false
  var worktree: Releash_Client_V1_WorkspaceWorktreeList? {
    repository.worktrees.items.first { $0.path == branch.worktreePath }
  }
  var removalTitle: String {
    branch.removalRequiresForce ? "変更またはロックがあるWorktreeを強制削除しますか？" : "Worktreeを削除しますか？"
  }
  var removalButtonTitle: String { branch.removalRequiresForce ? "強制削除" : "削除" }
  var canRemove: Bool {
    !branch.isMainWorktree && !branch.isDeleting
  }
  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
      HStack(spacing: UIStyle.rowSpacing) {
        Image(systemName: branch.isMainWorktree ? "house" : "arrow.triangle.branch")
          .frame(width: UIStyle.cardIconWidth)
        Text(branch.name).fontWeight(.medium)
      }
      VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
        if branch.isMerged || (branch.hasDirtyCount && branch.dirtyCount > 0) || branch.isDeleting {
          HStack {
            if branch.isMerged {
              Image(systemName: "checkmark.circle").foregroundStyle(.secondary)
                .accessibilityLabel("Merged").help("Merged")
            }
            if branch.hasDirtyCount && branch.dirtyCount > 0 {
              Text("\(branch.dirtyCount) changes").font(.caption).foregroundStyle(.secondary)
            }
            if branch.isDeleting {
              ProgressView().controlSize(.small)
              Text("削除中")
            }
          }
        }
        if branch.hasPrState {
          Image(systemName: prIcon(branch.prState))
            .foregroundStyle(.secondary)
            .help("PR #\(branch.prStateNumber): \(branch.prState)")
        }
        if branch.hasDirtyCountError { Text("Changes unavailable").help(branch.dirtyCountError) }
        if branch.hasPullRequestError { Text("PR unavailable").help(branch.pullRequestError) }
        if let worktree {
          if worktree.status.hasError { Text(worktree.status.error).foregroundStyle(.secondary) }
          ForEach(worktree.executions, id: \.id) { execution in
            VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
              if execution.isWorkflow {
                HStack(spacing: UIStyle.rowSpacing) {
                  Image("WorkflowWaypoints")
                    .resizable()
                    .renderingMode(.template)
                    .scaledToFit()
                    .frame(width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
                    .foregroundStyle(UIStyle.statusColor(execution.status))
                    .accessibilityLabel("Workflow, \(execution.status)")
                  Text("\(execution.title) · \(execution.nodeCount) nodes")
                }
                .foregroundStyle(.secondary)
                LazyVGrid(
                  columns: [
                    GridItem(.adaptive(minimum: UIStyle.statusSize), spacing: UIStyle.statusSpacing)
                  ], alignment: .leading, spacing: UIStyle.statusSpacing
                ) {
                  ForEach(Array(execution.sessionStates.enumerated()), id: \.offset) { _, state in
                    Rectangle().fill(UIStyle.statusColor(state)).frame(
                      width: UIStyle.statusSize, height: UIStyle.statusSize)
                  }
                }
                .padding(.leading, UIStyle.cardIconWidth + UIStyle.rowSpacing)
              } else {
                HStack(spacing: UIStyle.rowSpacing) {
                  Image("SessionBot")
                    .resizable()
                    .renderingMode(.template)
                    .scaledToFit()
                    .frame(width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
                    .foregroundStyle(UIStyle.statusColor(execution.status))
                    .accessibilityLabel("Session, \(execution.status)")
                  Text(
                    execution.provider.displayName
                  )
                  .fontWeight(.medium)
                  Text(execution.title).lineLimit(1)
                }
              }
            }
          }
        }
      }
      .padding(.leading, UIStyle.cardIconWidth + UIStyle.rowSpacing)
    }
    .font(.callout)
    .padding(.vertical, UIStyle.rowSpacing)
    .contentShape(Rectangle())
    .contextMenu {
      Button("削除", role: .destructive) { deleting = true }
        .disabled(!canRemove)
    }
    .confirmationDialog(
      removalTitle,
      isPresented: $deleting
    ) {
      Button(removalButtonTitle, role: .destructive) { remove() }
    }
  }

  private func prIcon(_ value: String) -> String {
    switch value {
    case "open": "arrow.triangle.pull"
    case "draft": "ellipsis.circle"
    case "merged": "arrow.triangle.merge"
    default: "xmark.circle"
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
      model.deselectWorktree(request.worktreePath)
    }
  }
}
