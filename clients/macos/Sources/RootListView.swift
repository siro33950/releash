import Bonsplit
import SwiftUI

extension Releash_Client_V1_WorkspaceTreeItem {
  var rootID: String {
    switch variant { case .node(let v): v.id; case .sequence(let v): v.id; case .fanout(let v): v.id; case nil: "" }
  }
  var rootTitle: String {
    switch variant { case .node(let v): v.title; case .sequence(let v): v.title; case .fanout(let v): v.title; case nil: "" }
  }
  var rootTabTitle: String {
    if case .node(let v) = variant, v.hasProvider { return "\(v.provider.displayName) \(v.title)" }
    return rootTitle
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
        Spacer()
        IconMenu(title: "その他", systemImage: "ellipsis") {
          Button("History…") { history = true }
        }
        IconMenu(title: "開始", systemImage: "plus") {
          Menu("New Session") {
            ForEach(model.providers, id: \.value) { provider in
              Button(provider.displayName) { model.perform { try await model.startSession(provider: provider.wireName, path: path) } }
            }
          }
          Menu("New Workflow") {
            ForEach(model.workflows, id: \.name) { workflow in
              Button(workflow.name) { model.perform { try await model.startWorkflow(name: workflow.name, path: path) } }
            }
          }.disabled(model.pendingRoots[path] != nil)
        }
      }.padding(UIStyle.spacing).disabled(!model.layoutLoaded)
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
            if node.capabilities.canRename { Button("Rename") { name = root.rootTitle; renaming = root.rootID } }
            if node.capabilities.canResumeSession { Button("Resume") { model.resumeNode(path: path, id: root.rootID) } }
          }
          if root.workflowCapabilities.canArchive {
            Button("Archive") {
              if root.workflowCapabilities.canAbort { archive = root.rootID } else { archiveRoot(root.rootID) }
            }
          }
        }
      }.listStyle(.sidebar).scrollContentBackground(.hidden)
    }
    .sheet(isPresented: $history) { SessionHistoryView(model: model, path: path) }
    .alert("実行をAbortしてArchiveしますか？", isPresented: Binding(get: { archive != nil }, set: { if !$0 { archive = nil } })) {
      Button("キャンセル", role: .cancel) { archive = nil }
      Button("Abort and Archive", role: .destructive) { if let root = archive { archiveRoot(root) }; archive = nil }
    }
    .alert("Rename", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
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
