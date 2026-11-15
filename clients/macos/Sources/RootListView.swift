import Bonsplit
import SwiftUI

extension Releash_Client_V1_WorkspaceTreeItem {
  var rootID: String {
    switch variant {
    case .node(let v): v.id
    case .sequence(let v): v.id
    case .fanout(let v): v.id
    case nil: ""
    }
  }
  var rootTitle: String {
    switch variant {
    case .node(let v): v.title
    case .sequence(let v): v.title
    case .fanout(let v): v.title
    case nil: ""
    }
  }
  var rootTabTitle: String {
    if case .node(let v) = variant, v.hasProvider { return "\(v.provider.displayName) \(v.title)" }
    return rootTitle
  }
  var rootKind: PaneTab.Kind? {
    switch variant {
    case .node(let v): v.contentKind.value == .command ? .command : .session
    default: nil
    }
  }
  var rootStatus: Releash_Client_V1_WorkspaceStatusClassification {
    switch variant {
    case .node(let v): v.status
    case .sequence(let v): v.status
    case .fanout(let v): v.status
    case nil: .init()
    }
  }
  var workflowCapabilities: Releash_Client_V1_WorkspaceWorkflowCapabilities {
    switch variant {
    case .node(let v): v.workflowCapabilities
    case .sequence(let v): v.workflowCapabilities
    case .fanout(let v): v.workflowCapabilities
    case nil: .init()
    }
  }
}

struct RootListView: View {
  @Bindable var model: AppModel
  let path: String
  @State private var selected: String?
  @State private var expandedRows: [String: Bool] = [:]
  @State private var expandedPastAttempts: Set<String> = []
  @State private var history = false
  @State private var archive: String?
  @State private var renaming: String?
  @State private var name = ""
  @State private var detailHeight: CGFloat?
  private static let splitSpace = "node-panel-split"

  var roots: [Releash_Client_V1_WorkspaceTreeItem] {
    model.repositories.flatMap(\.worktrees.items).first { $0.path == path }?.snapshot.nodes.items
      ?? []
  }
  private var defaultExpansion: [String: Bool] {
    var values: [String: Bool] = [:]
    func collect(_ items: [Releash_Client_V1_WorkspaceTreeItem]) {
      for item in items {
        if !item.children.isEmpty { values[item.rootID] = item.defaultExpanded }
        collect(item.children)
        collect(item.pastAttempts)
      }
    }
    collect(roots)
    return values
  }
  var body: some View {
    let rootIDs = Set(roots.map(\.rootID))
    let selectedItem = selected.flatMap { id in roots.lazy.compactMap { $0.find(id) }.first }
    GeometryReader { geometry in
      let bounds = Self.detailBounds(total: geometry.size.height)
      let lowerHeight = Self.detailHeight(total: geometry.size.height, chosen: detailHeight)
      VStack(spacing: 0) {
        HStack {
          Spacer()
          IconMenu(title: "More", image: "ellipsis") {
            Button("History…") { history = true }
          }
          SessionWorkflowMenu(model: model, path: path)
        }.padding(UIStyle.spacing).disabled(!model.layoutLoaded)
        if model.pendingRoots[path] != nil {
          ProgressView("Starting Workflow…").controlSize(.small)
        }
        Table(
          of: Releash_Client_V1_WorkspaceTreeItem.self,
          selection: Binding(
            get: { selected }, set: { selectionChanged(to: $0, by: NSApp.currentEvent) })
        ) {
          TableColumn("Node") { item in nameCell(item, isRoot: rootIDs.contains(item.rootID)) }
            .width(min: UIStyle.nodeNameMinimumWidth, ideal: UIStyle.nodeNameWidth)
          TableColumn("Elapsed") { item in missingCell(item, "Elapsed") }
            .width(UIStyle.nodeMetricWidth)
          TableColumn("Active") { item in missingCell(item, "Active") }
            .width(UIStyle.nodeMetricWidth)
          TableColumn("Token") { item in missingCell(item, "Token") }
            .width(UIStyle.nodeMetricWidth)
          TableColumn("Cost") { item in missingCell(item, "Cost") }
            .width(UIStyle.nodeMetricWidth)
          TableColumn("Model") { item in modelCell(item) }
            .width(min: UIStyle.nodeModelMinimumWidth, ideal: UIStyle.nodeModelWidth)
          TableColumn("Artifact") { item in artifactCell(item) }.width(UIStyle.nodeMetricWidth)
          TableColumn("Timeline") { item in Color.clear.modifier(RowActivation { open(item) }) }
            .width(min: UIStyle.nodeTimelineMinimumWidth, ideal: UIStyle.nodeTimelineWidth)
        } rows: {
          WorkspaceTableRows(
            items: roots, isExpanded: isExpanded, setExpanded: setExpanded,
            expandedPastAttempts: $expandedPastAttempts)
        }
        .scrollContentBackground(.hidden).alternatingRowBackgrounds(.disabled)
        .frame(maxHeight: .infinity)
        Divider().overlay {
          Color.clear.frame(height: UIStyle.dividerCursorWidth).contentShape(Rectangle())
            .gesture(
              DragGesture(coordinateSpace: .named(Self.splitSpace)).onChanged {
                detailHeight = geometry.size.height - $0.location.y
              }
            )
            .pointerStyle(.rowResize)
            .accessibilityRepresentation {
              Slider(
                value: Binding(get: { lowerHeight }, set: { detailHeight = $0 }),
                in: bounds.lowerBound...max(bounds.upperBound, bounds.lowerBound + 1)
              ) {
                Text("Detail height")
              }
              .accessibilityIdentifier("node-panel-divider")
            }
        }
        HStack(spacing: 0) {
          if let selected {
            NodeInspector(
              model: model, path: path, nodeID: selected,
              name: selectedItem?.displayName(isRoot: rootIDs.contains(selected))
            ).id(selected)
          } else {
            Text("No Node selected").foregroundStyle(.secondary)
              .frame(maxWidth: .infinity, maxHeight: .infinity)
          }
          Divider()
          NodeEvents().frame(width: geometry.size.width * UIStyle.nodeEventsWidthRatio)
        }
        .frame(height: lowerHeight)
      }
      .coordinateSpace(.named(Self.splitSpace))
    }
    .onChange(of: defaultExpansion) { oldValues, newValues in
      expandedRows = expandedRows.filter { newValues[$0.key] != nil }
      for (id, value) in newValues where oldValues[id] != nil && oldValues[id] != value {
        setExpanded(id, value)
      }
    }
    .sheet(isPresented: $history) { SessionHistoryView(model: model, path: path) }
    .alert(
      "Abort and archive this execution?",
      isPresented: Binding(get: { archive != nil }, set: { if !$0 { archive = nil } })
    ) {
      Button("Cancel", role: .cancel) { archive = nil }
      Button("Abort and Archive", role: .destructive) {
        if let root = archive { archiveRoot(root) }
        archive = nil
      }
    }
    .alert(
      "Rename", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })
    ) {
      TextField("Name", text: $name)
      Button("Cancel", role: .cancel) { renaming = nil }
      Button("Save") {
        guard let root = renaming, let client = model.client else { return }
        var request = Releash_Client_V1_RenameWorkspaceSessionNodeRequest()
        request.worktreePath = path
        request.nodeID = root
        request.name = name
        model.perform {
          _ = try await client.renameWorkspaceSessionNode(request: request, headers: model.headers)
            .result.get()
        }
        renaming = nil
      }
    }
  }
  /// 下段の高さの範囲。上段（右上の操作と表）と下段のどちらも最小の高さを割らない。両方は入らない高さでは上段を優先する
  static func detailBounds(total: CGFloat) -> ClosedRange<CGFloat> {
    let upper = max(total - UIStyle.nodeUpperMinimumHeight, 0)
    return min(UIStyle.nodeDetailMinimumHeight, upper)...upper
  }
  static func detailHeight(total: CGFloat, chosen: CGFloat?) -> CGFloat {
    let bounds = detailBounds(total: total)
    return min(
      max(chosen ?? total * UIStyle.nodeDetailHeightRatio, bounds.lowerBound), bounds.upperBound)
  }
  /// 表は、セルの中身の外（字下げや余白）のクリックを選択の変更として伝える。そのクリックでもタブを開き、
  /// キーボードやアクセシビリティで選択だけが動いたときは開かない
  func selectionChanged(
    to id: String?, by event: NSEvent?,
    now: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) {
    guard let id, let item = roots.lazy.compactMap({ $0.find(id) }).first else { return }
    if Self.isClick(event, now: now) { open(item) } else { selected = id }
  }
  static func isClick(
    _ event: NSEvent?, now: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) -> Bool {
    guard let event, event.type == .leftMouseDown || event.type == .leftMouseUp else {
      return false
    }
    return now - event.timestamp < 1
  }
  func missingCell(_ item: Releash_Client_V1_WorkspaceTreeItem, _ column: String) -> some View {
    Text(verbatim: "—").font(.caption.monospaced()).foregroundStyle(.secondary)
      .frame(maxWidth: .infinity, alignment: .trailing)
      .accessibilityLabel("\(item.rootTitle), \(column)")
      .modifier(RowActivation { open(item) })
  }
  func modelCell(_ item: Releash_Client_V1_WorkspaceTreeItem) -> some View {
    let model = item.nodeModel ?? "—"
    let text = Text(model).font(.caption.monospaced()).lineLimit(1).truncationMode(.middle)
      .frame(maxWidth: .infinity, alignment: .trailing)
    return HStack {
      if item.nodeModel == nil { text.foregroundStyle(.secondary) } else { text }
    }
    .accessibilityElement(children: .ignore).accessibilityIdentifier("node-model:\(item.rootID)")
    .accessibilityLabel("\(item.rootTitle), Model \(model)")
    .help(model)
    .modifier(RowActivation { open(item) })
  }
  func artifactCell(_ item: Releash_Client_V1_WorkspaceTreeItem) -> some View {
    HStack {
      Text(item.artifactLabel).font(.caption.monospaced()).foregroundStyle(Color.accentColor)
        .frame(maxWidth: .infinity, alignment: .trailing)
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("\(item.rootTitle), Artifact")
    .accessibilityIdentifier("node-artifact:\(item.rootID)").accessibilityAddTraits(.isStaticText)
    .accessibilityValue(item.artifactLabel)
    .modifier(RowActivation { open(item) })
  }
  func nameCell(_ item: Releash_Client_V1_WorkspaceTreeItem, isRoot: Bool) -> some View {
    HStack(spacing: UIStyle.rowSpacing) {
      Button {
        open(item)
      } label: {
        HStack(spacing: UIStyle.rowSpacing) {
          if let kind = item.kind {
            Image(kind.icon).resizable()
              .frame(width: UIStyle.statusSize, height: UIStyle.statusSize)
              .foregroundStyle(UIStyle.statusColor(String(describing: item.rootStatus.value)))
              .accessibilityLabel(kind.title)
          }
          Text(item.displayName(isRoot: isRoot)).lineLimit(1)
            .fontWeight(isRoot ? .semibold : .regular)
        }.contentShape(Rectangle())
      }
      .buttonStyle(.plain).layoutPriority(-1)
      .accessibilityIdentifier("root:\(item.rootID)")
      .accessibilityValue(String(describing: item.rootStatus.value))
      .contextMenu {
        if isRoot {
          if case .node(let node) = item.variant {
            if node.capabilities.canRename {
              Button("Rename") {
                name = item.rootTitle
                renaming = item.rootID
              }
            }
            if node.capabilities.canResumeSession {
              Button("Resume") { model.resumeNode(path: path, id: item.rootID) }
            }
          }
          if item.workflowCapabilities.canArchive {
            Button("Archive") {
              if item.workflowCapabilities.canAbort {
                archive = item.rootID
              } else {
                archiveRoot(item.rootID)
              }
            }
          }
        }
      }
      if !item.pastAttempts.isEmpty {
        Button {
          if expandedPastAttempts.contains(item.rootID) {
            expandedPastAttempts.remove(item.rootID)
          } else {
            expandedPastAttempts.insert(item.rootID)
          }
        } label: {
          HStack(spacing: UIStyle.statusSpacing) {
            CardIcon(image: Image("history"))
            Text(verbatim: "\(item.pastAttempts.count + 1)").font(.caption)
          }
        }
        .buttonStyle(.borderless).help("Past attempts").accessibilityLabel("Past attempts")
        .accessibilityValue("\(item.pastAttempts.count + 1)")
        .accessibilityIdentifier("past-attempts:\(item.rootID)")
      }
      if item.canApprove {
        Button("Approve") { approve(item.rootID) }
          .buttonStyle(.borderedProminent).controlSize(.small)
          .accessibilityIdentifier("approve:\(item.rootID)")
      }
      if let reason = item.errorReason {
        CardIcon(image: Image("circle-alert")).foregroundStyle(UIStyle.failure).help(reason)
          .accessibilityLabel("Failed").accessibilityValue(reason)
          .accessibilityIdentifier("node-failure:\(item.rootID)")
      }
    }.frame(maxWidth: .infinity, alignment: .leading)
      .modifier(RowActivation { open(item) })
  }
  func open(_ item: Releash_Client_V1_WorkspaceTreeItem) {
    selected = item.rootID
    model.openRoot(item)
  }
  private func isExpanded(_ item: Releash_Client_V1_WorkspaceTreeItem) -> Bool {
    expandedRows[item.rootID] ?? item.defaultExpanded
  }
  private func setExpanded(_ id: String, _ value: Bool) {
    expandedRows[id] = value
  }
  func approve(_ id: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ApproveWorkspaceNodeRequest()
    request.worktreePath = path
    request.nodeID = id
    model.perform {
      _ = try await client.approveWorkspaceNode(request: request, headers: model.headers).result
        .get()
    }
  }
  func archiveRoot(_ rootID: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest()
    request.worktreePath = path
    request.executionID = rootID
    model.perform {
      _ = try await client.archiveWorkspaceWorkflowExecution(
        request: request, headers: model.headers
      ).result.get()
    }
  }
}

private struct RowActivation: ViewModifier {
  let action: () -> Void

  func body(content: Content) -> some View {
    content.frame(maxHeight: .infinity).contentShape(Rectangle()).onTapGesture(perform: action)
  }
}

private struct NodeEvents: View {
  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      Text("Events").font(.caption).foregroundStyle(.secondary).padding(UIStyle.spacing)
      Divider()
      Text("No events").foregroundStyle(.secondary).padding(UIStyle.spacing)
        .accessibilityIdentifier("node-events-empty")
      Spacer(minLength: 0)
    }.frame(maxWidth: .infinity, alignment: .leading)
  }
}

extension Releash_Client_V1_WorkspaceTreeItem {
  var defaultExpanded: Bool { rootStatus.value != .idle }
  var kind: UIStyle.NodeKind? {
    switch variant {
    case .node(let node): node.contentKind.value == .command ? .command : .session
    case .sequence: .sequence
    case .fanout: .fanout
    case nil: nil
    }
  }
  func displayName(isRoot: Bool) -> String { isRoot ? rootTabTitle : rootTitle }
  var children: [Self] {
    switch variant {
    case .node(let node): node.children.items
    case .sequence(let node): node.children.items
    case .fanout(let node): node.children.items
    case nil: []
    }
  }
  var pastAttempts: [Self] {
    guard case .node(let node) = variant else { return [] }
    return node.pastAttempts.items.compactMap { attempt in
      guard case .node(let value) = attempt.variant else { return nil }
      var item = Self()
      item.node = value
      return item
    }
  }
  var nodeModel: String? {
    guard case .node(let node) = variant else { return "" }
    return kind?.modelLabel ?? (node.model.isEmpty ? nil : node.model)
  }
  var artifactLabel: String {
    guard case .node(let node) = variant, node.artifactCount > 0 else { return "" }
    return node.artifactCount == 1 ? "◆" : "◆×\(node.artifactCount)"
  }
  var errorReason: String? {
    switch variant {
    case .node(let node): node.hasErrorReason ? node.errorReason : nil
    case .sequence(let node): node.hasErrorReason ? node.errorReason : nil
    case .fanout(let node): node.hasErrorReason ? node.errorReason : nil
    case nil: nil
    }
  }
  var canApprove: Bool {
    switch variant {
    case .node(let node): node.capabilities.canApprove
    case .sequence(let node): node.canApprove
    case .fanout(let node): node.canApprove
    case nil: false
    }
  }
}

private struct WorkspaceTableRows: TableRowContent {
  typealias TableRowValue = Releash_Client_V1_WorkspaceTreeItem
  let items: [Releash_Client_V1_WorkspaceTreeItem]
  let isExpanded: (Releash_Client_V1_WorkspaceTreeItem) -> Bool
  let setExpanded: (String, Bool) -> Void
  @Binding var expandedPastAttempts: Set<String>

  @TableRowBuilder<Releash_Client_V1_WorkspaceTreeItem>
  var tableRowBody: some TableRowContent<Releash_Client_V1_WorkspaceTreeItem> {
    ForEach(items, id: \.rootID) { (item: Releash_Client_V1_WorkspaceTreeItem) in
      if item.children.isEmpty {
        TableRow(item)
      } else {
        DisclosureTableRow(
          item,
          isExpanded: Binding(
            get: { isExpanded(item) },
            set: { setExpanded(item.rootID, $0) })
        ) {
          WorkspaceTableRows(
            items: item.children, isExpanded: isExpanded, setExpanded: setExpanded,
            expandedPastAttempts: $expandedPastAttempts
          )
        }
      }
      if expandedPastAttempts.contains(item.rootID) {
        WorkspaceTableRows(
          items: item.pastAttempts, isExpanded: isExpanded, setExpanded: setExpanded,
          expandedPastAttempts: $expandedPastAttempts)
      }
    }
  }
}

extension Releash_Client_V1_WorkspaceTreeItem: Identifiable {
  public var id: String { rootID }
  func find(_ id: String) -> Self? {
    if rootID == id { return self }
    return (children + pastAttempts).lazy.compactMap { $0.find(id) }.first
  }
}
