import Bonsplit
import SwiftUI

extension Releash_Client_V1_WorkspaceTreeItem {
  var rootID: String {
    switch variant { case .node(let v): v.id; case .sequence(let v): v.id; case .fanout(let v): v.id; case nil: "" }
  }
  var rootTitle: String {
    switch variant { case .node(let v): v.title; case .sequence(let v): v.title; case .fanout(let v): v.title; case nil: "" }
  }
  var rootIcon: String {
    switch variant { case .sequence: "arrow.right"; case .fanout: "arrow.triangle.branch"; default: rootKind.icon }
  }
  var rootKind: PaneTab.Kind {
    switch variant { case .node(let v): v.contentKind.value == .command ? .command : .session; default: .workflow }
  }
  var rootStatus: Releash_Client_V1_WorkspaceStatusClassification {
    switch variant { case .node(let v): v.status; case .sequence(let v): v.status; case .fanout(let v): v.status; case nil: .init() }
  }
  var workflowCapabilities: Releash_Client_V1_WorkspaceWorkflowCapabilities {
    switch variant { case .node(let v): v.workflowCapabilities; case .sequence(let v): v.workflowCapabilities; case .fanout(let v): v.workflowCapabilities; case nil: .init() }
  }
}

struct RootListView: View {
  @Bindable var model: AppModel
  let path: String
  @State private var launch: PaneOpenMenu.LaunchKind?
  @State private var history = false
  @State private var archive: String?
  @State private var renaming: String?
  @State private var name = ""

  var roots: [Releash_Client_V1_WorkspaceTreeItem] {
    model.repositories.flatMap(\.worktrees.items).first { $0.path == path }?.snapshot.nodes.items ?? []
  }
  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Button("New Session") { launch = .session }
        Button("New Workflow") { launch = .workflow }.disabled(model.pendingRoots[path] != nil)
        Button("History") { history = true }
      }.buttonStyle(.accessoryBar).padding(UIStyle.spacing).disabled(!model.layoutLoaded)
      if model.pendingRoots[path] != nil { ProgressView("Rootの配信を待っています").controlSize(.small) }
      List(roots, id: \.rootID) { root in
        Button { model.openRoot(root) } label: {
          HStack(spacing: UIStyle.rowSpacing) {
            Group {
              if root.rootKind == .session { Image("SessionBot").resizable().renderingMode(.template) }
              else { Image(systemName: root.rootIcon).resizable() }
            }.scaledToFit().frame(width: UIStyle.cardIconWidth, height: UIStyle.cardIconWidth)
              .foregroundStyle(UIStyle.statusColor(String(describing: root.rootStatus.value)))
              .accessibilityLabel(root.rootKind.title)
            if case .node(let node) = root.variant, node.hasProvider {
              Text(node.provider.displayName).fontWeight(.medium)
            }
            Text(root.rootTitle).lineLimit(1)
          }.frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityIdentifier("root:\(root.rootID)")
        .accessibilityValue(String(describing: root.rootStatus.value))
        .contextMenu {
          if case .node(let node) = root.variant {
            if node.capabilities.canRename { Button("改名") { name = root.rootTitle; renaming = root.rootID } }
            if node.capabilities.canResumeSession { Button("Resume") { model.resumeNode(path: path, id: root.rootID) } }
          }
          if root.workflowCapabilities.canArchive {
            Button("Archive") {
              if root.workflowCapabilities.canAbort { archive = root.rootID } else { archiveRoot(root.rootID) }
            }
          }
        }
      }
    }
    .sheet(item: $launch) { kind in
      if let pane = model.panes.controller.focusedPaneId { PaneLaunchView(model: model, pane: pane, path: path, kind: kind) }
    }
    .sheet(isPresented: $history) { SessionHistoryView(model: model, path: path) }
    .alert("実行を中止してArchiveしますか？", isPresented: Binding(get: { archive != nil }, set: { if !$0 { archive = nil } })) {
      Button("キャンセル", role: .cancel) { archive = nil }
      Button("中止してArchive", role: .destructive) { if let root = archive { archiveRoot(root) }; archive = nil }
    }
    .alert("改名", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
      TextField("名前", text: $name)
      Button("キャンセル", role: .cancel) { renaming = nil }
      Button("保存") {
        guard let root = renaming, let client = model.client else { return }
        var request = Releash_Client_V1_RenameWorkspaceSessionNodeRequest()
        request.worktreePath = path; request.nodeID = root; request.name = name
        model.perform { _ = try await client.renameWorkspaceSessionNode(request: request, headers: model.headers).result.get() }
        renaming = nil
      }
    }
  }
  func archiveRoot(_ rootID: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest()
    request.worktreePath = path; request.executionID = rootID
    model.perform { _ = try await client.archiveWorkspaceWorkflowExecution(request: request, headers: model.headers).result.get() }
  }
}

extension AppModel {
  func openRoot(_ root: Releash_Client_V1_WorkspaceTreeItem) {
    panes.openNode(root.rootKind, id: root.rootID, title: root.rootTitle)
    savePaneLayout()
  }
  func resumeNode(path: String, id: String) {
    guard let client else { return }
    var request = Releash_Client_V1_ResumeWorkspaceSessionNodeRequest()
    request.worktreePath = path; request.nodeID = id
    perform { _ = try await client.resumeWorkspaceSessionNode(request: request, headers: self.headers).result.get() }
  }
}
