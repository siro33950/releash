import Bonsplit
import SwiftUI

@MainActor @Observable final class PaneWorkspace: @preconcurrency BonsplitDelegate {
  var requestClose: ((PaneTab) async -> Bool)?
  private var closing: Set<TabID> = []
  private var approved: Set<TabID> = []
  private var approvedPanes: Set<PaneID> = []
  private var closingPanes: Set<PaneID> = []
  let controller = BonsplitController()
  private var identifiers: [String: String] = [:]
  private var tabs: [String: PaneTab] = [:]
  private var nextTerminalNumber = 1

  init() {
    for id in controller.allTabIds { controller.closeTab(id) }
    controller.delegate = self
    controller.configuration.appearance.showSplitButtons = false
    controller.configuration.appearance.selectedTabIndicatorColor = UIStyle.tabSelection
  }

  func restore(_ layout: PaneLayout) throws {
    var ids: [String: String] = [:]
    var restoredTabs: [String: PaneTab] = [:]
    var metadata: [Bonsplit.Tab] = []
    var terminalNumber = 1
    func identifier(_ original: String) -> String {
      var id = (UUID(uuidString: original) ?? UUID()).uuidString
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
          let title = tab.kind == .terminal ? "Terminal \(terminalNumber)" : tab.kind.title
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
    try controller.restoreLayout(snapshot, tabs: metadata)
    identifiers = ids
    self.tabs = restoredTabs
    nextTerminalNumber = terminalNumber
  }

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

  func tab(_ id: TabID) -> PaneTab? { tabs[id.id.uuidString] }

  func splitTabBar(_ controller: BonsplitController, shouldCloseTab tab: Bonsplit.Tab, inPane pane: PaneID) -> Bool {
    if approved.remove(tab.id) != nil { return true }
    guard let value = tabs[tab.id.id.uuidString], value.kind == .terminal, let requestClose else { return true }
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
    let terminalTabs = controller.tabs(inPane: pane).compactMap { tabs[$0.id.id.uuidString] }.filter { $0.kind == .terminal }
    guard !terminalTabs.isEmpty, let requestClose else { return true }
    guard closingPanes.insert(pane).inserted else { return false }
    Task {
      defer { closingPanes.remove(pane) }
      while let tab = controller.tabs(inPane: pane).first(where: { tabs[$0.id.id.uuidString]?.kind == .terminal }), let value = tabs[tab.id.id.uuidString] {
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
      if model.layoutLoaded, let value = model.panes.tab(tab.id), value.kind == .terminal, let path = model.selectedWorktree {
        TerminalPane(session: model.terminal(path: path, id: value.id), focused: model.panes.controller.focusedPaneId == pane).id(path + value.id)
      } else {
        Text(tab.title).foregroundStyle(.secondary)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    } emptyPane: { _ in
      Color(nsColor: .textBackgroundColor)
    }
    .onAppear {
      model.panes.requestClose = { [weak model] tab in await model?.closeTerminal(tab) ?? false }
      model.panes.controller.tabBarAccessory = { [weak model] pane in
        guard let model else { return AnyView(EmptyView()) }
        return AnyView(PaneOpenMenu(model: model, pane: pane))
      }
    }
    .onChange(of: model.panes.controller.treeSnapshot()) { _, _ in
      guard model.layoutLoaded else { return }
      do {
        let layout = try model.panes.layout()
        model.editLayout { $0 = layout }
        model.syncTerminalTabs(layout)
      } catch { model.failure = error.localizedDescription }
    }
  }
}
