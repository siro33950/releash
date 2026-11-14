import SwiftUI

extension Releash_Client_V1_WorkspaceRepositoryList {
  fileprivate var sidebarID: String { "repository:\(path)" }
}

struct SidebarView: View {
  @Bindable var model: AppModel
  @Binding var creating: Bool
  @State private var sidebarWidth = UIStyle.sidebarWidth
  func repositoryFailure(_ repository: Releash_Client_V1_WorkspaceRepositoryList) -> String? {
    repository.status.hasError ? repository.status.error : nil
  }
  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Spacer()
        Button {
          creating = true
        } label: {
          CardIcon(image: Image("plus"))
        }.buttonStyle(IconButtonStyle()).iconButtonFrame().help("Worktreeを作成").accessibilityLabel(
          "Worktreeを作成")
      }
      .padding(.horizontal, UIStyle.spacing)
      .frame(height: UIStyle.tabBarHeight)
      Divider()
      List {
        ForEach(model.repositories, id: \.sidebarID) { repository in
          Section(
            isExpanded: Binding(
              get: { !(model.collapsed[repository.path] ?? false) },
              set: { model.setCollapsed(repository.path, !$0) }
            )
          ) {
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
              .accessibilityIdentifier("worktree:\(branch.name)")
              .buttonStyle(.plain)
              .foregroundStyle(
                model.selectedWorktree == branch.worktreePath
                  ? UIStyle.sidebarSelectionText : .primary
              )
              .accessibilityAddTraits(
                model.selectedWorktree == branch.worktreePath ? .isSelected : [])
            }
          } header: {
            HStack(spacing: UIStyle.rowSpacing) {
              Text(URL(fileURLWithPath: repository.path).lastPathComponent)
                .accessibilityIdentifier("repository:\(repository.path)")
              Text(repository.branches.items.count, format: .number)
                .foregroundStyle(.secondary)
              Spacer()
              if let error = repositoryFailure(repository) {
                WarningBadge(message: error)
                  .padding(.trailing, UIStyle.rowSpacing)
              }
            }
            // 開閉ボタンの出現で警告が移動しないよう、その幅を常に空ける。
            .frame(
              width: max(0, sidebarWidth - UIStyle.spacing * 2 - UIStyle.iconButtonWidth),
              alignment: .leading
            )
            .selectionDisabled()
          }
        }
      }
      .listStyle(.sidebar)
      .scrollContentBackground(.hidden)
      Divider()
      HStack {
        Spacer()
        Button(action: model.addRepository) {
          CardIcon(image: Image("folder-plus"))
        }.buttonStyle(IconButtonStyle()).iconButtonFrame().help("Repositoryを追加")
          .accessibilityLabel("Repositoryを追加")
        Button(action: model.refresh) {
          CardIcon(image: Image("refresh-cw"))
        }.buttonStyle(IconButtonStyle()).iconButtonFrame().help("更新").accessibilityLabel("更新")
      }
      .padding(.horizontal, UIStyle.spacing)
      .frame(height: UIStyle.tabBarHeight)
    }
    .onGeometryChange(for: CGFloat.self) {
      $0.size.width
    } action: {
      sidebarWidth = $0
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
        CardIcon(image: Image(branch.isMainWorktree ? "house" : "git-branch"))
        Text(branch.name).fontWeight(.medium).lineLimit(1)
        Spacer()
        if branch.hasPrState {
          let appearance = prAppearance(branch.prState)
          CardIcon(image: Image(appearance.icon))
            .foregroundStyle(appearance.color)
            .help("PR #\(branch.prStateNumber): \(branch.prState)")
        }
        if let warning {
          WarningBadge(message: warning)
        }
      }
      VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
        if branch.isDeleting {
          HStack(spacing: UIStyle.rowSpacing) {
            ProgressView().controlSize(.small)
            Text("削除中")
          }
        }
        if let worktree {
          ForEach(worktree.executions, id: \.id) { execution in
            HStack(spacing: UIStyle.rowSpacing) {
              CardIcon(image: Image(execution.isWorkflow ? "waypoints" : "bot"))
                .foregroundStyle(UIStyle.statusColor(execution.status))
                .accessibilityLabel(
                  "\(execution.isWorkflow ? "Workflow" : "Session"), \(execution.status)")
              if execution.isWorkflow {
                (Text(execution.title)
                  + Text(" · \(execution.nodeCount) nodes").foregroundColor(.secondary))
                  .lineLimit(1)
              } else {
                Text(execution.title).lineLimit(1)
              }
            }
          }
        }
      }
      .frame(
        minHeight: NSLayoutManager().defaultLineHeight(for: .preferredFont(forTextStyle: .callout)),
        alignment: .topLeading
      )
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

  private var warning: String? {
    let messages = [
      branch.hasDirtyCountError ? "Changes unavailable: \(branch.dirtyCountError)" : nil,
      branch.hasPullRequestError ? "PR unavailable: \(branch.pullRequestError)" : nil,
      worktree.flatMap { $0.status.hasError ? $0.status.error : nil },
    ].compactMap { $0 }
    return messages.isEmpty ? nil : messages.joined(separator: "\n")
  }

  private func prAppearance(_ value: String) -> (icon: String, color: Color) {
    switch value {
    case "open": ("git-pull-request", .green)
    case "draft": ("git-pull-request-draft", .gray)
    case "merged": ("git-merge", .purple)
    default: ("git-pull-request-closed", .red)
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

private struct WarningBadge: View {
  let message: String

  var body: some View {
    CardIcon(image: Image("triangle-alert"))
      .foregroundStyle(.secondary)
      // List 内でもクリックを透過して表示するため、ツールチップは AppKit が持つ。
      .accessibilityHint(message)
      .overlay {
        WarningTooltip(message: message)
          .allowsHitTesting(false)
          .accessibilityHidden(true)
      }
  }
}

private struct WarningTooltip: NSViewRepresentable {
  let message: String

  func makeNSView(context: Context) -> TooltipView { TooltipView() }

  func updateNSView(_ view: TooltipView, context: Context) {
    view.toolTip = message
  }

  final class TooltipView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
  }
}
