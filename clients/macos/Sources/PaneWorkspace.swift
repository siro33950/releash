import Bonsplit
import SwiftUI

@MainActor @Observable final class PaneWorkspace {
  let controller = BonsplitController()
  private var identifiers: [String: String] = [:]
  private var tabs: [String: PaneTab] = [:]

  init() {
    for id in controller.allTabIds { controller.closeTab(id) }
    controller.configuration.appearance.showSplitButtons = false
    controller.configuration.appearance.selectedTabIndicatorColor = UIStyle.tabSelection
  }

  func restore(_ layout: PaneLayout) throws {
    var ids: [String: String] = [:]
    var restoredTabs: [String: PaneTab] = [:]
    var metadata: [Bonsplit.Tab] = []
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
          metadata.append(
            Bonsplit.Tab(
              id: TabID(id: UUID(uuidString: id)!), title: tab.kind.title, icon: tab.kind.icon))
          if tab.id == active { selected = id }
          return ExternalTab(id: id, title: tab.kind.title)
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
    guard let id = controller.createTab(title: kind.title, icon: kind.icon, inPane: pane) else {
      return
    }
    tabs[id.id.uuidString] = PaneTab(id: id.id.uuidString, kind: kind)
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
    BonsplitView(controller: model.panes.controller) { tab, _ in
      Text(tab.title).foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    } emptyPane: { _ in
      Color(nsColor: .textBackgroundColor)
    }
    .onAppear {
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
      } catch { model.failure = error.localizedDescription }
    }
  }
}
