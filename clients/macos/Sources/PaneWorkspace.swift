import Bonsplit
import SwiftUI

@MainActor @Observable final class PaneWorkspace: @preconcurrency BonsplitDelegate {
  var sessionSnapshots: [TabID: Releash_Client_V1_SessionTabSnapshot] = [:]
  var renamingSession: Releash_Client_V1_SessionTabSnapshot?
  var archivingSession: Releash_Client_V1_SessionTabSnapshot?
  var sessionName = ""
  var requestClose: ((PaneTab) async -> Bool)?
  private var closing: Set<TabID> = []
  private var approved: Set<TabID> = []
  private var approvedPanes: Set<PaneID> = []
  private var closingPanes: Set<PaneID> = []
  let controller = BonsplitController()
  private var identifiers: [String: String] = [:]
  private var tabs: [String: PaneTab] = [:]
  private var pendingSessionTabs: [String: TabID] = [:]
  private var nextTerminalNumber = 1

  init() {
    for id in controller.allTabIds { controller.closeTab(id) }
    controller.delegate = self
    controller.configuration.appearance.showSplitButtons = false
    controller.configuration.contentViewLifecycle = .keepAllAlive
    controller.configuration.appearance.selectedTabIndicatorColor = UIStyle.tabSelection
  }

  func restore(_ layout: PaneLayout) throws {
    if try applyToExistingPanes(layout) { return }
    var ids: [String: String] = [:]
    var restoredTabs: [String: PaneTab] = [:]
    var metadata: [Bonsplit.Tab] = []
    var terminalNumber = 1
    func identifier(_ original: String) -> String {
      var id =
        UUID(uuidString: original)?.uuidString
        ?? identifiers.first(where: { $0.value == original })?.key
        ?? UUID().uuidString
      if ids[id] != nil { id = UUID().uuidString }
      ids[id] = original
      return id
    }
    func tree(_ layout: PaneLayout) throws -> ExternalTreeNode {
      switch layout {
      case .pane(let id, let tabs, let active):
        guard active == nil || tabs.contains(where: { $0.id == active }) else {
          throw CLIFailure(message: "Missing active pane tab")
        }
        var selected: String?
        let values = tabs.map { tab -> ExternalTab in
          let id = identifier(tab.id)
          restoredTabs[id] = tab
          let title = tab.kind == .terminal ? "Terminal \(terminalNumber)" : tab.title
          if tab.kind == .terminal { terminalNumber += 1 }
          metadata.append(
            Bonsplit.Tab(
              id: TabID(id: UUID(uuidString: id)!), title: title, icon: tab.kind.icon))
          if tab.id == active { selected = id }
          return ExternalTab(id: id, title: title)
        }
        return .pane(
          ExternalPaneNode(
            id: identifier(id),
            frame: PixelRect(x: 0, y: 0, width: 0, height: 0), tabs: values, selectedTabId: selected
          ))
      case .split(let id, let axis, let ratio, let first, let second):
        return .split(
          ExternalSplitNode(
            id: identifier(id), orientation: axis.rawValue,
            dividerPosition: ratio, first: try tree(first), second: try tree(second)))
      }
    }
    let snapshot = try tree(layout)
    try restoreLayoutPreservingFocus(snapshot, tabs: metadata)
    identifiers = ids
    self.tabs = restoredTabs
    pendingSessionTabs = [:]
    nextTerminalNumber = terminalNumber
  }

  /// pane の構造が今と同じなら、pane の状態を作り直さずにタブの差分だけを適用する。
  /// Bonsplit は復元で作り直した pane の状態を表示中の view へ渡さないため、
  /// サーバがタブを増減した配置はこの経路で反映する。構造が違えば false を返す。
  private func applyToExistingPanes(_ layout: PaneLayout) throws -> Bool {
    var pairs: [(pane: PaneID, tabs: [PaneTab], active: String?)] = []
    var dividers: [(split: UUID, ratio: Double, current: Double)] = []
    func match(_ current: ExternalTreeNode, _ target: PaneLayout) -> Bool {
      switch (current, target) {
      case (.pane(let pane), .pane(let id, let tabs, let active)):
        guard (identifiers[pane.id] ?? pane.id) == id,
          let paneID = UUID(uuidString: pane.id).map({ PaneID(id: $0) })
        else { return false }
        pairs.append((paneID, tabs, active))
        return true
      case (.split(let split), .split(let id, let axis, let ratio, let first, let second)):
        guard (identifiers[split.id] ?? split.id) == id, split.orientation == axis.rawValue,
          let splitID = UUID(uuidString: split.id)
        else { return false }
        dividers.append((splitID, ratio, split.dividerPosition))
        return match(split.first, first) && match(split.second, second)
      default: return false
      }
    }
    guard match(controller.treeSnapshot(), layout) else { return false }
    for (_, tabs, active) in pairs {
      guard active == nil || tabs.contains(where: { $0.id == active }) else {
        throw CLIFailure(message: "Missing active pane tab")
      }
    }
    let targetPanes = Dictionary(
      pairs.flatMap { pair in pair.tabs.map { ($0.id, pair.pane) } },
      uniquingKeysWith: { first, _ in first })
    for tab in controller.allTabIds {
      let original = identifiers[tab.id.uuidString] ?? tab.id.uuidString
      guard
        let pane = controller.allPaneIds.first(where: {
          controller.tabs(inPane: $0).contains { $0.id == tab }
        })
      else { continue }
      if let target = targetPanes[original], target != pane { return false }
    }
    for (pane, targetTabs, _) in pairs {
      let current = controller.tabs(inPane: pane).map {
        identifiers[$0.id.id.uuidString] ?? $0.id.id.uuidString
      }
      let kept = current.filter { id in targetTabs.contains { $0.id == id } }
      let expected = targetTabs.map(\.id).filter { current.contains($0) }
      if kept != expected { return false }
    }
    let focusedPane = controller.focusedPaneId
    let newTabPosition = controller.configuration.newTabPosition
    controller.configuration.newTabPosition = .end
    defer { controller.configuration.newTabPosition = newTabPosition }
    for divider in dividers where divider.current != divider.ratio {
      controller.setDividerPosition(
        CGFloat(divider.ratio), forSplit: divider.split, fromExternal: true)
    }
    for (pane, targetTabs, active) in pairs {
      let selectedBefore = controller.selectedTab(inPane: pane)?.id
      var byOriginal: [String: TabID] = [:]
      for tab in controller.tabs(inPane: pane) {
        let key = tab.id.id.uuidString
        let original = identifiers[key] ?? key
        if targetTabs.contains(where: { $0.id == original }) {
          byOriginal[original] = tab.id
        } else {
          approved.insert(tab.id)
          controller.closeTab(tab.id, preservingPane: true)
          identifiers.removeValue(forKey: key)
        }
      }
      for target in targetTabs {
        if let id = byOriginal[target.id] {
          if let old = tabs[id.id.uuidString],
            old.kind != target.kind || old.nodeID != target.nodeID
          {
            pendingSessionTabs = pendingSessionTabs.filter { $0.value != id }
          }
          tabs[id.id.uuidString] = target
          continue
        }
        let title = target.kind == .terminal ? "Terminal \(nextTerminalNumber)" : target.title
        if target.kind == .terminal { nextTerminalNumber += 1 }
        guard let id = controller.createTab(title: title, icon: target.kind.icon, inPane: pane)
        else { continue }
        identifiers[id.id.uuidString] = target.id
        tabs[id.id.uuidString] = target
        byOriginal[target.id] = id
      }
      let desired = active.flatMap { byOriginal[$0] } ?? selectedBefore
      if let desired, controller.allTabIds.contains(desired),
        controller.selectedTab(inPane: pane)?.id != desired
      {
        controller.selectTab(desired)
      }
    }
    if let focusedPane, controller.allPaneIds.contains(focusedPane) {
      controller.focusPane(focusedPane)
    }
    pendingSessionTabs = pendingSessionTabs.filter { controller.allTabIds.contains($0.value) }
    return true
  }

  /// レイアウトを復元し、復元前にフォーカスされていた pane を保つ。
  private func restoreLayoutPreservingFocus(
    _ snapshot: ExternalTreeNode, tabs: [Bonsplit.Tab]
  ) throws {
    let focusedPane = controller.focusedPaneId
    try controller.restoreLayout(snapshot, tabs: tabs)
    if let focusedPane, controller.allPaneIds.contains(focusedPane) {
      controller.focusPane(focusedPane)
    }
  }

  struct LayoutInput: Equatable {
    var tree: ExternalTreeNode
    var tabs: [String: PaneTab]

    static func == (lhs: Self, rhs: Self) -> Bool {
      lhs.tabs == rhs.tabs && samePlacement(lhs.tree, rhs.tree)
    }

    private static func samePlacement(_ lhs: ExternalTreeNode, _ rhs: ExternalTreeNode) -> Bool {
      switch (lhs, rhs) {
      case (.pane(let lhs), .pane(let rhs)):
        lhs.id == rhs.id && lhs.tabs.map(\.id) == rhs.tabs.map(\.id)
          && lhs.selectedTabId == rhs.selectedTabId
      case (.split(let lhs), .split(let rhs)):
        lhs.id == rhs.id && lhs.orientation == rhs.orientation
          && lhs.dividerPosition == rhs.dividerPosition
          && samePlacement(lhs.first, rhs.first) && samePlacement(lhs.second, rhs.second)
      default: false
      }
    }
  }

  var layoutInput: LayoutInput { LayoutInput(tree: controller.treeSnapshot(), tabs: tabs) }

  func layout() throws -> PaneLayout {
    func convert(_ tree: ExternalTreeNode) throws -> PaneLayout {
      switch tree {
      case .pane(let pane):
        let values = try pane.tabs.map { value -> PaneTab in
          guard let tab = tabs[value.id] else { throw CLIFailure(message: "Unknown pane tab") }
          return tab
        }
        return .pane(
          id: identifiers[pane.id] ?? pane.id, tabs: values,
          active: pane.selectedTabId.map { identifiers[$0] ?? $0 })
      case .split(let split):
        return .split(
          id: identifiers[split.id] ?? split.id,
          axis: split.orientation == "horizontal" ? .horizontal : .vertical,
          ratio: split.dividerPosition, first: try convert(split.first),
          second: try convert(split.second))
      }
    }
    return try convert(controller.treeSnapshot())
  }

  func open(_ kind: PaneTab.Kind, in pane: PaneID) {
    let live = Set(controller.allTabIds.map { $0.id.uuidString })
    tabs = tabs.filter { live.contains($0.key) }
    let title = kind == .terminal ? "Terminal \(nextTerminalNumber)" : kind.title
    guard let id = controller.createTab(title: title, icon: kind.icon, inPane: pane) else {
      return
    }
    let tab = PaneTab(id: id.id.uuidString, kind: kind)
    tabs[id.id.uuidString] = tab
    if kind == .terminal { nextTerminalNumber += 1 }
  }

  @discardableResult
  func openNode(_ kind: PaneTab.Kind, id nodeID: String, title: String, in pane: PaneID? = nil)
    -> TabID?
  {
    let live = Set(controller.allTabIds.map { $0.id.uuidString })
    tabs = tabs.filter { live.contains($0.key) }
    if let existing = controller.allTabIds.first(where: {
      tabs[$0.id.uuidString]?.kind == kind
        && (kind == .session
          ? tabs[$0.id.uuidString]?.agentSessionID : tabs[$0.id.uuidString]?.nodeID) == nodeID
    }
    ) {
      controller.selectTab(existing)
      return existing
    }
    guard
      let id = controller.createTab(
        title: title, icon: kind.icon, inPane: pane ?? controller.focusedPaneId)
    else { return nil }
    tabs[id.id.uuidString] = PaneTab(
      id: id.id.uuidString, kind: kind, nodeID: kind == .session ? nil : nodeID,
      agentSessionID: kind == .session ? nodeID : nil)
    return id
  }

  func openSession(id sessionID: String, nodeID: String, title: String) {
    if let pending = controller.allTabIds.first(where: {
      tabs[$0.id.uuidString]?.kind == .session && tabs[$0.id.uuidString]?.nodeID == nodeID
    }) {
      if let selected = bindSession(pending, sessionID: sessionID) {
        controller.selectTab(selected)
      }
    } else {
      let selected = openNode(.session, id: sessionID, title: title)
      if let selected {
        pendingSessionTabs[nodeID] = selected
      }
    }
  }

  func openPendingSession(id nodeID: String, title: String) {
    pendingSessionTabs = pendingSessionTabs.filter { controller.allTabIds.contains($0.value) }
    if let existing = pendingSessionTabs[nodeID]
      ?? controller.allTabIds.first(where: {
        tabs[$0.id.uuidString]?.kind == .session && tabs[$0.id.uuidString]?.nodeID == nodeID
      })
    {
      controller.selectTab(existing)
      return
    }
    guard let id = controller.createTab(title: title, icon: PaneTab.Kind.session.icon) else {
      return
    }
    tabs[id.id.uuidString] = PaneTab(id: id.id.uuidString, kind: .session, nodeID: nodeID)
    pendingSessionTabs[nodeID] = id
  }

  @discardableResult
  func bindSession(_ id: TabID, sessionID: String) -> TabID? {
    guard controller.allTabIds.contains(id),
      var tab = tabs[id.id.uuidString], tab.kind == .session
    else { return nil }
    if let nodeID = tab.nodeID { pendingSessionTabs[nodeID] = id }
    if let existing = controller.allTabIds.first(where: {
      $0 != id && tabs[$0.id.uuidString]?.kind == .session
        && tabs[$0.id.uuidString]?.agentSessionID == sessionID
    }) {
      pendingSessionTabs = pendingSessionTabs.mapValues { $0 == id ? existing : $0 }
      controller.closeTab(id, preservingPane: true)
      controller.selectTab(existing)
      return existing
    }
    tab.agentSessionID = sessionID
    tab.nodeID = nil
    tabs[id.id.uuidString] = tab
    return id
  }

  func splitTabBar(_ controller: BonsplitController, didCloseTab id: TabID, fromPane pane: PaneID) {
    tabs.removeValue(forKey: id.id.uuidString)
    pendingSessionTabs = pendingSessionTabs.filter { $0.value != id }
  }

  func renameNode(_ nodeID: String, title: String) {
    for id in controller.allTabIds
    where tabs[id.id.uuidString]?.kind == .command && tabs[id.id.uuidString]?.nodeID == nodeID {
      controller.updateTab(id, title: title)
    }
  }

  func updateSession(_ id: TabID, snapshot: Releash_Client_V1_SessionTabSnapshot) {
    sessionSnapshots[id] = snapshot
    controller.updateTab(
      id, title: snapshot.hasNode ? snapshot.node.title : "Session",
      iconColor: .some(
        snapshot.hasNode
          ? UIStyle.statusColor(String(describing: snapshot.node.classification.value)) : nil))
  }

  func isFocused(_ tab: TabID, in pane: PaneID) -> Bool {
    controller.focusedPaneId == pane && controller.selectedTab(inPane: pane)?.id == tab
  }

  func tab(_ id: TabID) -> PaneTab? { tabs[id.id.uuidString] }

  func splitTabBar(
    _ controller: BonsplitController, shouldCloseTab tab: Bonsplit.Tab, inPane pane: PaneID
  ) -> Bool {
    if approved.remove(tab.id) != nil { return true }
    guard let value = tabs[tab.id.id.uuidString], value.kind == .terminal, let requestClose else {
      return true
    }
    guard closing.insert(tab.id).inserted else { return false }
    Task {
      defer { closing.remove(tab.id) }
      if await requestClose(value) {
        approved.insert(tab.id)
        controller.closeTab(tab.id)
      }
    }
    return false
  }

  func splitTabBar(_ controller: BonsplitController, shouldClosePane pane: PaneID) -> Bool {
    if approvedPanes.remove(pane) != nil { return true }
    let terminalTabs = controller.tabs(inPane: pane).compactMap { tabs[$0.id.id.uuidString] }.filter
    { $0.kind == .terminal }
    guard !terminalTabs.isEmpty, let requestClose else { return true }
    guard closingPanes.insert(pane).inserted else { return false }
    Task {
      defer { closingPanes.remove(pane) }
      while let tab = controller.tabs(inPane: pane).first(where: {
        tabs[$0.id.id.uuidString]?.kind == .terminal
      }), let value = tabs[tab.id.id.uuidString] {
        guard await requestClose(value) else { return }
        approved.insert(tab.id)
        controller.closeTab(tab.id)
      }
      if controller.allPaneIds.contains(pane) {
        approvedPanes.insert(pane)
        controller.closePane(pane)
      }
    }
    return false
  }

  func openFile(_ path: String, base: String) {
    guard let pane = controller.focusedPaneId ?? controller.allPaneIds.first else { return }
    if let existing = controller.tabs(inPane: pane).first(where: {
      tabs[$0.id.id.uuidString]?.filePath == path
    }) {
      controller.selectTab(existing.id)
      return
    }
    guard
      let id = controller.createTab(
        title: URL(fileURLWithPath: path).lastPathComponent, icon: PaneTab.Kind.file.icon,
        inPane: pane)
    else { return }
    tabs[id.id.uuidString] = PaneTab(
      id: id.id.uuidString, kind: .file, filePath: path, fileBase: base)
  }

  func setFilePresentation(_ id: TabID, mode: String?, base: String?) {
    guard var tab = tabs[id.id.uuidString] else { return }
    if let mode { tab.fileMode = mode }
    if let base { tab.fileBase = base }
    tabs[id.id.uuidString] = tab
  }

  func closeCurrentTab() {
    guard let pane = controller.focusedPaneId, let tab = controller.selectedTab(inPane: pane) else {
      return
    }
    controller.closeTab(tab.id)
  }

  func closeOtherTabs() {
    guard let pane = controller.focusedPaneId else { return }
    let active = controller.selectedTab(inPane: pane)?.id
    for tab in controller.tabs(inPane: pane) where tab.id != active { controller.closeTab(tab.id) }
  }
}

struct WorkspacePaneView: View {
  @Bindable var model: AppModel
  var body: some View {
    BonsplitView(controller: model.panes.controller) { tab, pane in
      if model.layoutLoaded, let value = model.panes.tab(tab.id), value.kind == .terminal,
        let path = model.selectedWorktree
      {
        TerminalPane(
          session: model.terminal(path: path, id: value.id),
          focused: model.panes.isFocused(tab.id, in: pane)
        ).id(path + value.id)
      } else if model.layoutLoaded, let value = model.panes.tab(tab.id),
        let sessionID = value.agentSessionID, value.kind == .session
      {
        SessionTabPane(
          model: model, tabID: tab.id, sessionID: sessionID,
          focused: model.panes.isFocused(tab.id, in: pane)
        ).id(sessionID)
      } else if model.layoutLoaded, let value = model.panes.tab(tab.id), let nodeID = value.nodeID,
        let path = model.selectedWorktree, value.kind == .session || value.kind == .command
      {
        NodePane(
          model: model, path: path, nodeID: nodeID,
          focused: model.panes.isFocused(tab.id, in: pane),
          tabID: tab.id
        ).id(path + nodeID)
      } else if let file = model.panes.tab(tab.id), let path = file.filePath,
        let worktree = model.selectedWorktree
      {
        FileTabPane(model: model, tabID: tab.id, path: path, worktree: worktree).id(
          "\(worktree):\(tab.id):\(path)")
      } else {
        Text(tab.title).foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    } emptyPane: { _ in
      Color(nsColor: .textBackgroundColor)
    }
    .onAppear {
      model.panes.requestClose = { [weak model] tab in await model?.closeTerminal(tab) ?? false }
      model.panes.controller.tabContextMenu = { [weak model] id in
        guard let model else { return AnyView(EmptyView()) }
        return AnyView(SessionTabMenu(model: model, tabID: id))
      }
      model.panes.controller.tabBarAccessory = { [weak model] pane in
        guard let model else { return AnyView(EmptyView()) }
        return AnyView(PaneOpenMenu(model: model, pane: pane))
      }
    }
    .alert(
      "Rename",
      isPresented: Binding(
        get: { model.panes.renamingSession != nil },
        set: { if !$0 { model.panes.renamingSession = nil } })
    ) {
      TextField("名前", text: $model.panes.sessionName)
      Button("キャンセル", role: .cancel) { model.panes.renamingSession = nil }
      Button("保存") {
        if let snapshot = model.panes.renamingSession {
          model.renameSessionTab(snapshot, name: model.panes.sessionName)
        }
        model.panes.renamingSession = nil
      }
    }
    .alert(
      "実行をAbortしてArchiveしますか？",
      isPresented: Binding(
        get: { model.panes.archivingSession != nil },
        set: { if !$0 { model.panes.archivingSession = nil } })
    ) {
      Button("キャンセル", role: .cancel) { model.panes.archivingSession = nil }
      Button("Abort and Archive", role: .destructive) {
        model.confirmArchiveSessionTab()
      }
    }
    .onChange(of: model.panes.layoutInput) { _, _ in
      saveLayout()
    }

  }

  private func saveLayout() {
    guard model.layoutLoaded else { return }
    do {
      let layout = try model.panes.layout()
      model.editLayout { $0 = layout }
      model.syncTerminalTabs(layout)
    } catch { model.failure = error.localizedDescription }
  }
}
