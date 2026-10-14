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
  var rootKind: PaneTab.Kind? {
    switch variant { case .node(let v): v.contentKind.value == .command ? .command : .session; default: nil }
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
  @State private var treeViewportWidth: CGFloat?
  @State private var selected: String?
  @State private var expanded: Set<String> = []
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
      VSplitView {
        GeometryReader { geometry in
          let fixedWidth: CGFloat = 80 + 24 + UIStyle.spacing * 4
          let nameWidth = max(240, (treeViewportWidth ?? geometry.size.width) - UIStyle.scrollerSpace - fixedWidth)
          let width = nameWidth + fixedWidth
          ScrollView([.horizontal, .vertical]) {
            LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
              Section {
                ForEach(roots, id: \.rootID) { root in
                  treeRow(root, depth: 0, isRoot: true, nameWidth: nameWidth)
                }
              } header: {
                VStack(spacing: 0) {
                  HStack(spacing: UIStyle.spacing) {
                    Text("実行木").frame(width: nameWidth, alignment: .leading)
                    Text("MODEL").frame(width: 80, alignment: .trailing)
                    Text("ART").frame(width: 24)
                  }.font(.caption).foregroundStyle(.secondary).padding(.vertical, UIStyle.rowSpacing)
                  Divider()
                }.background(Color(nsColor: .textBackgroundColor))
              }
            }.padding(.horizontal, UIStyle.spacing).frame(minWidth: width, alignment: .leading)
          }
          .defaultScrollAnchor(.topLeading, for: .alignment)
          .onScrollGeometryChange(for: CGFloat.self) { $0.visibleRect.width } action: { _, width in treeViewportWidth = width }
        }.frame(minHeight: 120)
        if let selected {
          NodeInspector(model: model, path: path, nodeID: selected).id(selected).frame(minHeight: 150)
        } else {
          Text("Nodeを選択").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
      }
    }
    .background(Color(nsColor: .textBackgroundColor))
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
  func treeRow(_ item: Releash_Client_V1_WorkspaceTreeItem, depth: Int, isRoot: Bool, nameWidth: CGFloat) -> AnyView {
    AnyView(VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .top, spacing: UIStyle.spacing) {
        HStack(spacing: UIStyle.rowSpacing) {
          Button {
          if item.rootKind != nil { selected = item.rootID }
          else { toggle(item.rootID) }
        } label: {
          HStack(spacing: UIStyle.rowSpacing) {
            Image(systemName: "circle.fill").font(.system(size: UIStyle.statusSize)).foregroundStyle(UIStyle.statusColor(String(describing: item.rootStatus.value)))
              .accessibilityLabel(item.rootKind?.title ?? item.compositeKind)
            if item.rootKind == nil { Image(systemName: expanded.contains(item.rootID) ? "chevron.down" : "chevron.right") }
            if isRoot, case .node(let node) = item.variant, node.hasProvider { Text(node.provider.displayName).fontWeight(.medium) }
            VStack(alignment: .leading) {
              HStack(spacing: UIStyle.rowSpacing) {
                Text(item.rootTitle).lineLimit(1).fontWeight(item.rootKind == nil ? .semibold : .regular)
                Text(item.rootKind?.title ?? item.compositeKind)
                  .font(.caption2).foregroundStyle(.secondary).padding(.horizontal, UIStyle.rowSpacing)
                  .background(Color(nsColor: .quaternarySystemFill), in: RoundedRectangle(cornerRadius: 3))
              }
              if case .node(let node) = item.variant, node.hasErrorReason { Text(node.errorReason).font(.caption).lineLimit(1) }
              if let worktree = item.isolatedWorktree { Text(worktree.branch + " · " + worktree.path).font(.caption).foregroundStyle(.secondary) }
            }
          }.padding(.leading, CGFloat(depth) * UIStyle.padding).contentShape(Rectangle())
        }.buttonStyle(.plain).accessibilityIdentifier("root:\(item.rootID)")
          .accessibilityValue(String(describing: item.rootStatus.value))
          .contextMenu {
            if isRoot {
              if case .node(let node) = item.variant {
                if node.capabilities.canRename { Button("Rename") { name = item.rootTitle; renaming = item.rootID } }
                if node.capabilities.canResumeSession { Button("Resume") { model.resumeNode(path: path, id: item.rootID) } }
              }
              if item.workflowCapabilities.canArchive {
                Button("Archive") {
                  if item.workflowCapabilities.canAbort { archive = item.rootID } else { archiveRoot(item.rootID) }
                }
              }
            }
          }
          if item.rootKind != nil {
            Button("▷") { model.openRoot(item) }.buttonStyle(.bordered).controlSize(.mini)
              .accessibilityLabel("タブを開く").accessibilityIdentifier("open-node:\(item.rootID)")
          }
          if item.canApprove { Button("Approve") { approve(item.rootID) }.controlSize(.small).accessibilityIdentifier("approve:\(item.rootID)") }
          if !item.pastAttempts.isEmpty {
            Button { toggle("attempts:" + item.rootID) } label: {
              Image(systemName: expanded.contains("attempts:" + item.rootID) ? "chevron.down" : "chevron.right")
            }.buttonStyle(.borderless).help("過去の試行").accessibilityLabel("過去の試行")
          }
          Spacer(minLength: 0).contentShape(Rectangle()).onTapGesture { select(item) }
        }.frame(width: nameWidth, alignment: .leading)
        Button { select(item) } label: {
          Text(item.nodeModel).font(.system(.caption, design: .monospaced)).frame(width: 80, alignment: .trailing)
        }.buttonStyle(.plain).accessibilityIdentifier("node-model:\(item.rootID)")
          .accessibilityLabel("\(item.rootTitle)を選択、MODEL \(item.nodeModel)")
        Button { select(item) } label: {
          Text(item.hasArtifact ? "◆" : "").foregroundStyle(Color.accentColor).frame(width: 24)
        }.buttonStyle(.plain).accessibilityLabel("\(item.rootTitle)を選択、Artifact\(item.hasArtifact ? "あり" : "なし")")
          .accessibilityIdentifier("node-artifact:\(item.rootID)").accessibilityValue(item.hasArtifact ? "◆" : "")
      }.padding(.vertical, UIStyle.rowSpacing)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(selected == item.rootID ? UIStyle.sidebarSelection : .clear)
        .foregroundStyle(selected == item.rootID ? UIStyle.sidebarSelectionText : .primary)
      Divider()
      if item.rootKind != nil || expanded.contains(item.rootID) {
        ForEach(item.children, id: \.rootID) { child in treeRow(child, depth: depth + 1, isRoot: false, nameWidth: nameWidth) }
      }
      if expanded.contains("attempts:" + item.rootID) {
        ForEach(item.pastAttempts, id: \.rootID) { attempt in treeRow(attempt, depth: depth + 1, isRoot: false, nameWidth: nameWidth) }
      }
    })
  }
  func select(_ item: Releash_Client_V1_WorkspaceTreeItem) {
    if item.rootKind != nil { selected = item.rootID }
    else { toggle(item.rootID) }
  }
  func toggle(_ id: String) {
    if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
  }
  func approve(_ id: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ApproveWorkspaceNodeRequest()
    request.worktreePath = path; request.nodeID = id
    model.perform { _ = try await client.approveWorkspaceNode(request: request, headers: model.headers).result.get() }
  }
  func archiveRoot(_ rootID: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest()
    request.worktreePath = path; request.executionID = rootID
    model.perform { _ = try await client.archiveWorkspaceWorkflowExecution(request: request, headers: model.headers).result.get() }
  }
}

extension Releash_Client_V1_WorkspaceTreeItem {
  var compositeKind: String { if case .fanout = variant { return "FANOUT" }; return "SEQ" }
  var children: [Self] {
    switch variant { case .node(let node): node.children.items; case .sequence(let node): node.children.items; case .fanout(let node): node.children.items; case nil: [] }
  }
  var pastAttempts: [Self] {
    guard case .node(let node) = variant else { return [] }
    return node.pastAttempts.items.compactMap { attempt in
      guard case .node(let value) = attempt.variant else { return nil }
      var item = Self(); item.node = value; return item
    }
  }
  var nodeModel: String { if case .node(let node) = variant { return node.model }; return "" }
  var hasArtifact: Bool { if case .node(let node) = variant { return node.hasArtifact_p }; return false }
  var canApprove: Bool {
    switch variant { case .node(let node): node.capabilities.canApprove; case .sequence(let node): node.canApprove; case .fanout(let node): node.canApprove; case nil: false }
  }
  var isolatedWorktree: Releash_Client_V1_NodeWorktreeDto? {
    switch variant { case .sequence(let node): node.hasWorktree ? node.worktree : nil; case .fanout(let node): node.hasWorktree ? node.worktree : nil; default: nil }
  }
}
