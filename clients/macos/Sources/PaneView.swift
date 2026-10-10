import SwiftUI
import UniformTypeIdentifiers

private let paneTabType = UTType.utf8PlainText

struct PaneView: View {
  let layout: PaneLayout
  @Bindable var model: AppModel
  var body: some View {
    switch layout {
    case .split(let id, let axis, let ratio, let first, let second):
      SplitPaneView(id: id, axis: axis, ratio: ratio, first: first, second: second, model: model)
    case .pane(let id, let tabs, let active):
      TabbedPane(id: id, tabs: tabs, active: active, model: model)
    }
  }
}

private struct TabbedPane: View {
  let id: String
  let tabs: [PaneTab]
  let active: String?
  @Bindable var model: AppModel
  @State private var dropTargeted = false
  @State private var dropEdge: PaneLayout.Edge?

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 0) {
        ScrollViewReader { proxy in
          ScrollView(.horizontal) {
            HStack(spacing: 0) {
              ForEach(tabs) { tab in
                PaneTabView(tab: tab, pane: id, tabs: tabs, active: active == tab.id, model: model)
                  .id(tab.id)
              }
              Color.clear.frame(minWidth: UIStyle.tabControlWidth, maxWidth: .infinity)
                .onDrop(of: [paneTabType], delegate: TabDrop(model: model, pane: id))
            }
          }
          .scrollIndicators(.never)
          .onChange(of: active, initial: true) { _, active in
            if let active { proxy.scrollTo(active) }
          }
          .onChange(of: tabs) { _, _ in
            if let active { proxy.scrollTo(active) }
          }
        }
        Menu {
          ForEach(PaneTab.Kind.allCases, id: \.self) { kind in
            Button(kind.title) { model.openTab(kind, in: id) }
          }
        } label: {
          Image(systemName: "plus")
        }
        .menuIndicator(.hidden).menuStyle(.borderlessButton)
        .fixedSize().frame(width: UIStyle.tabControlWidth)
        .help("新しいタブ").accessibilityLabel("新しいタブ")
        Menu {
          ForEach(tabs) { tab in
            Button {
              model.selectTab(tab.id, in: id)
            } label: {
              if active == tab.id {
                Label(tab.kind.title, systemImage: "checkmark")
              } else {
                Text(tab.kind.title)
              }
            }
          }
        } label: {
          Image(systemName: "chevron.down")
        }
        .menuIndicator(.hidden).menuStyle(.borderlessButton)
        .fixedSize().frame(width: UIStyle.tabControlWidth)
        .disabled(tabs.isEmpty).help("タブを選択").accessibilityLabel("タブを選択")
      }
      .frame(height: UIStyle.tabBarHeight)
      .background(Color(nsColor: .windowBackgroundColor))
      .background(alignment: .bottom) { Divider() }
      GeometryReader { geometry in
        ZStack {
          if let tab = tabs.first(where: { $0.id == active }) {
            ContentUnavailableView(tab.kind.title, systemImage: "rectangle")
          } else {
            Color.clear
          }
          if dropTargeted {
            Rectangle().fill(Color.accentColor.opacity(UIStyle.dropOpacity))
              .overlay(Rectangle().strokeBorder(Color.accentColor, lineWidth: UIStyle.tabBorder))
              .frame(
                width: dropEdge == .left || dropEdge == .right ? geometry.size.width / 2 : nil,
                height: dropEdge == .top || dropEdge == .bottom ? geometry.size.height / 2 : nil
              )
              .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: dropAlignment)
              .allowsHitTesting(false)
          }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
        .contentShape(Rectangle())
        .onTapGesture { model.focusPane(id) }
        .onDrop(
          of: [paneTabType],
          delegate: PaneDrop(
            model: model, pane: id, size: geometry.size, targeted: $dropTargeted, edge: $dropEdge))
      }
    }
    .frame(minWidth: UIStyle.minimumPaneSize, minHeight: UIStyle.minimumPaneSize)
    .onChange(of: model.paneInteraction.draggedTab) { _, tab in
      if tab == nil {
        dropTargeted = false
        dropEdge = nil
      }
    }
  }
  private var dropAlignment: Alignment {
    switch dropEdge {
    case .left: .leading
    case .right: .trailing
    case .top: .top
    case .bottom: .bottom
    case nil: .center
    }
  }
}

private struct PaneTabView: View {
  let tab: PaneTab
  let pane: String
  let tabs: [PaneTab]
  let active: Bool
  @Bindable var model: AppModel
  @State private var hovered = false
  @State private var targeted = false

  var body: some View {
    HStack(spacing: UIStyle.tabSpacing) {
      DraggableTabButton(
        title: tab.kind.title, selected: active,
        emphasized: active && model.focusedPane?.id == pane,
        select: { model.selectTab(tab.id, in: pane) },
        begin: { model.paneInteraction.draggedTab = tab.id },
        end: {
          model.paneInteraction.draggedTab = nil
          hovered = false
        },
        tabID: tab.id, type: paneTabType.identifier
      )
      .fixedSize(horizontal: true, vertical: false)
      .frame(height: UIStyle.tabBarHeight)
      Button {
        model.closeTabs([tab.id])
      } label: {
        Image(systemName: "xmark").font(.system(size: UIStyle.tabFontSize))
          .frame(width: UIStyle.tabCloseSize, height: UIStyle.tabCloseSize)
          .contentShape(Rectangle())
      }
      .buttonStyle(.plain).opacity(hovered ? 1 : 0)
      .accessibilityLabel("\(tab.kind.title)を閉じる").help("タブを閉じる (⌘W)")
    }
    .padding(.leading, UIStyle.tabBorder)
    .padding(.trailing, UIStyle.tabPadding + UIStyle.tabBorder)
    .frame(height: UIStyle.tabBarHeight)
    .foregroundStyle(active && model.focusedPane?.id == pane ? Color.primary : Color.secondary)
    .background(Color(nsColor: active ? .textBackgroundColor : .windowBackgroundColor))
    .overlay(alignment: .trailing) { Divider() }
    .overlay(alignment: .bottom) { if !active { Divider() } }
    .overlay(alignment: insertAfter ? .trailing : .leading) {
      if targeted { Rectangle().fill(Color.accentColor).frame(width: UIStyle.tabInsertionBorder) }
    }
    .contentShape(Rectangle())
    .onHover { hovered = $0 }
    .onChange(of: model.paneInteraction.draggedTab) { _, tab in
      if tab == nil { targeted = false }
    }
    .help(tab.kind.title)
    .background(TabMiddleClick { model.closeTabs([tab.id]) })
    .onDrop(
      of: [paneTabType],
      delegate: TabDrop(
        model: model, pane: pane, target: tab.id, targeted: $targeted)
    )
    .contextMenu {
      Button("閉じる") { model.closeTabs([tab.id]) }
      Button("ほかのタブを閉じる") { model.closeTabs(tabs.filter { $0.id != tab.id }.map(\.id)) }
        .disabled(tabs.count < 2)
      Divider()
      Button("左側のタブを閉じる") { model.closeTabs(tabs.prefix(index).map(\.id)) }
        .disabled(index == 0)
      Button("右側のタブを閉じる") { model.closeTabs(tabs.dropFirst(index + 1).map(\.id)) }
        .disabled(index == tabs.count - 1)
      Button("すべて閉じる") { model.closeTabs(tabs.map(\.id)) }
    }
  }
  private var index: Int { tabs.firstIndex { $0.id == tab.id } ?? 0 }
  private var insertAfter: Bool {
    guard let dragged = model.paneInteraction.draggedTab,
      let source = model.layout.paneContaining(dragged),
      let sourceIndex = source.tabs.firstIndex(where: { $0.id == dragged })
    else { return false }
    return index > sourceIndex
  }
}

@MainActor private struct TabDrop: @preconcurrency DropDelegate {
  let model: AppModel
  let pane: String
  var target: String?
  var targeted: Binding<Bool>?
  func validateDrop(info: DropInfo) -> Bool {
    model.paneInteraction.draggedTab.flatMap { model.layout.tab($0) } != nil
  }
  func dropEntered(info: DropInfo) { targeted?.wrappedValue = true }
  func dropExited(info: DropInfo) { targeted?.wrappedValue = false }
  func dropUpdated(info: DropInfo) -> DropProposal? {
    DropProposal(operation: NSEvent.modifierFlags.contains(.option) ? .copy : .move)
  }
  func performDrop(info: DropInfo) -> Bool {
    targeted?.wrappedValue = false
    guard let dragged = model.paneInteraction.draggedTab,
      let source = model.layout.paneContaining(dragged), let destination = model.layout.pane(pane)
    else { return false }
    var before = target
    if source.id == pane, let target,
      let to = destination.tabs.firstIndex(where: { $0.id == target }),
      let from = source.tabs.firstIndex(where: { $0.id == dragged }), to > from
    {
      before = destination.tabs.dropFirst(to + 1).first?.id
    }
    let copy = NSEvent.modifierFlags.contains(.option)
    if dragged != target || copy { model.moveTab(dragged, to: pane, before: before, copy: copy) }
    model.paneInteraction.draggedTab = nil
    return true
  }
}

@MainActor private struct PaneDrop: @preconcurrency DropDelegate {
  let model: AppModel
  let pane: String
  let size: CGSize
  @Binding var targeted: Bool
  @Binding var edge: PaneLayout.Edge?
  func validateDrop(info: DropInfo) -> Bool {
    model.paneInteraction.draggedTab.flatMap { model.layout.tab($0) } != nil
  }
  func dropEntered(info: DropInfo) {
    targeted = true
    update(info.location)
  }
  func dropExited(info: DropInfo) {
    targeted = false
    edge = nil
  }
  func dropUpdated(info: DropInfo) -> DropProposal? {
    update(info.location)
    return DropProposal(operation: NSEvent.modifierFlags.contains(.option) ? .copy : .move)
  }
  private func update(_ position: CGPoint) {
    let distances: [(PaneLayout.Edge, CGFloat)] = [
      (.top, position.y), (.right, size.width - position.x),
      (.bottom, size.height - position.y), (.left, position.x),
    ]
    let nearest = distances.min { $0.1 < $1.1 }
    edge = nearest.flatMap {
      $0.1 < min(size.width, size.height) * UIStyle.splitDropFraction ? $0.0 : nil
    }
  }
  func performDrop(info: DropInfo) -> Bool {
    defer {
      targeted = false
      edge = nil
    }
    guard let tab = model.paneInteraction.draggedTab else { return false }
    update(info.location)
    model.moveTab(
      tab, to: pane, edge: edge,
      before: edge == nil ? model.layout.pane(pane)?.activeTab : nil,
      copy: NSEvent.modifierFlags.contains(.option))
    return true
  }
}
private struct SplitPaneView: View {
  let id: String
  let axis: PaneLayout.Axis
  let ratio: Double
  let first: PaneLayout
  let second: PaneLayout
  @Bindable var model: AppModel
  @State private var draggingRatio: Double?

  var body: some View {
    GeometryReader { geometry in
      let horizontal = axis == .horizontal
      let length = (horizontal ? geometry.size.width : geometry.size.height) - UIStyle.spacing
      let firstMinimum = first.minimumLength(along: axis)
      let secondMinimum = second.minimumLength(along: axis)
      let minimum = firstMinimum / max(length, firstMinimum + secondMinimum)
      let maximum = max(minimum, 1 - secondMinimum / max(length, 1))
      let fraction = min(maximum, max(minimum, draggingRatio ?? ratio))
      let stack =
        horizontal ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
      stack {
        AnyView(PaneView(layout: first, model: model))
          .frame(
            width: horizontal ? length * fraction : nil,
            height: horizontal ? nil : length * fraction)
        Divider()
          .frame(
            width: horizontal ? UIStyle.spacing : nil, height: horizontal ? nil : UIStyle.spacing
          )
          .contentShape(Rectangle())
          .gesture(
            DragGesture(coordinateSpace: .global).onChanged { drag in
              guard length > 0 else { return }
              let delta = horizontal ? drag.translation.width : drag.translation.height
              let initial = min(maximum, max(minimum, ratio))
              draggingRatio = min(maximum, max(minimum, initial + delta / length))
            }.onEnded { _ in
              if let value = draggingRatio { model.editLayout { $0.resize(id, ratio: value) } }
              draggingRatio = nil
            }
          )
          .onHover { inside in
            if inside {
              (horizontal ? NSCursor.resizeLeftRight : NSCursor.resizeUpDown).push()
            } else {
              NSCursor.pop()
            }
          }
          .accessibilityLabel("分割のサイズ")
          .accessibilityAdjustableAction { direction in
            let value = min(
              maximum, max(minimum, fraction + (direction == .increment ? 0.05 : -0.05)))
            model.editLayout { $0.resize(id, ratio: value) }
          }
        AnyView(PaneView(layout: second, model: model))
          .frame(
            width: horizontal ? length * (1 - fraction) : nil,
            height: horizontal ? nil : length * (1 - fraction))
      }
    }
  }
}
