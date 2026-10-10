import Foundation

struct PaneInteraction {
  var focusedPane: String?
  var history: [String] = []
  var draggedTab: String?
  var switcherTabs: [String] = []
  var switcherIndex = 0
  var switcherOriginal: String?

  mutating func activate(_ tab: String) {
    history.removeAll { $0 == tab }
    history.append(tab)
  }
  func recentTabs(in pane: PaneLayout) -> [String] {
    let ids = pane.tabs.map(\.id)
    let recent = history.reversed().filter { ids.contains($0) }
    return recent + ids.filter { !recent.contains($0) }
  }
  func replacement(for tab: String, in pane: PaneLayout) -> String? {
    if let recent = recentTabs(in: pane).first(where: { $0 != tab && history.contains($0) }) {
      return recent
    }
    guard let index = pane.tabs.firstIndex(where: { $0.id == tab }) else { return nil }
    return index > 0 ? pane.tabs[index - 1].id : pane.tabs.dropFirst().first?.id
  }
}

extension AppModel {
  var focusedPane: PaneLayout? {
    paneInteraction.focusedPane.flatMap { layout.pane($0) } ?? layout.panes.first
  }
  func focusPane(_ pane: String) {
    guard let value = layout.pane(pane) else { return }
    paneInteraction.focusedPane = pane
    if let active = value.activeTab { paneInteraction.activate(active) }
  }
  func selectTab(_ tab: String, in pane: String) {
    guard layout.pane(pane)?.tabs.contains(where: { $0.id == tab }) == true else { return }
    paneInteraction.switcherTabs = []
    paneInteraction.switcherOriginal = nil
    editLayout { $0.select(tab, in: pane) }
    paneInteraction.focusedPane = pane
    paneInteraction.activate(tab)
  }
  func openTab(_ kind: PaneTab.Kind, in pane: String) {
    focusPane(pane)
    editLayout { $0.open(kind, in: pane) }
    focusPane(pane)
  }
  func closeTabs(_ ids: [String]) {
    editLayout { layout in
      for id in ids {
        guard let pane = layout.paneContaining(id) else { continue }
        let replacement = paneInteraction.replacement(for: id, in: pane)
        layout.close(id)
        if pane.activeTab == id, let replacement {
          layout.select(replacement, in: pane.id)
        }
        paneInteraction.history.removeAll { $0 == id }
      }
    }
    if let pane = focusedPane { focusPane(pane.id) }
  }
  func moveTab(
    _ tab: String, to pane: String, edge: PaneLayout.Edge? = nil, before: String? = nil,
    copy: Bool = false
  ) {
    let source = layout.paneContaining(tab)
    let replacement = source.flatMap { paneInteraction.replacement(for: tab, in: $0) }
    var movedTab = tab
    editLayout { layout in
      if copy {
        movedTab = layout.duplicate(tab, to: pane, edge: edge) ?? tab
        return
      }
      layout.move(tab, to: pane, edge: edge, before: before)
      if let source, source.id != layout.paneContaining(tab)?.id,
        source.activeTab == tab, let replacement
      {
        layout.select(replacement, in: source.id)
      }
    }
    if let destination = layout.paneContaining(movedTab) { selectTab(movedTab, in: destination.id) }
    paneInteraction.draggedTab = nil
  }
  func cycleTab(_ direction: Int) {
    guard let pane = focusedPane, !pane.tabs.isEmpty,
      let index = pane.tabs.firstIndex(where: { $0.id == pane.activeTab })
    else { return }
    let next = (index + direction + pane.tabs.count) % pane.tabs.count
    selectTab(pane.tabs[next].id, in: pane.id)
  }
  func cycleRecentTab(_ direction: Int) {
    guard let pane = focusedPane, !pane.tabs.isEmpty else { return }
    if paneInteraction.switcherTabs.isEmpty {
      if let active = pane.activeTab { paneInteraction.activate(active) }
      paneInteraction.switcherTabs = paneInteraction.recentTabs(in: pane)
      paneInteraction.switcherOriginal = pane.activeTab
      paneInteraction.switcherIndex = 0
    }
    let count = paneInteraction.switcherTabs.count
    paneInteraction.switcherIndex = (paneInteraction.switcherIndex + direction + count) % count
    let tab = paneInteraction.switcherTabs[paneInteraction.switcherIndex]
    editLayout { $0.select(tab, in: pane.id) }
  }
  func finishTabSwitcher(cancel: Bool = false) {
    guard !paneInteraction.switcherTabs.isEmpty else { return }
    let tab =
      cancel
      ? paneInteraction.switcherOriginal
      : paneInteraction.switcherTabs[paneInteraction.switcherIndex]
    paneInteraction.switcherTabs = []
    paneInteraction.switcherOriginal = nil
    if let tab, let pane = layout.paneContaining(tab) { selectTab(tab, in: pane.id) }
  }
}
