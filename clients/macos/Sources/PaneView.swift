import SwiftUI

struct PaneView: View {
  let layout: PaneLayout
  @Bindable var model: AppModel
  var body: some View {
    switch layout {
    case .split(let id, let axis, let ratio, let first, let second):
      SplitPaneView(id: id, axis: axis, ratio: ratio, first: first, second: second, model: model)
    case .pane(let id, let tabs, let active):
      GeometryReader { geometry in
        VStack(spacing: 0) {
          HStack(spacing: UIStyle.spacing) {
            ForEach(tabs) { tab in
              HStack {
                Button(tab.kind.title) { model.editLayout { $0.select(tab.id, in: id) } }
                  .buttonStyle(.plain).fontWeight(active == tab.id ? .semibold : .regular)
                  .draggable(tab.id)
                Button {
                  model.editLayout { $0.close(tab.id) }
                } label: {
                  Image(systemName: "xmark")
                }
                .buttonStyle(.plain).accessibilityLabel("\(tab.kind.title)を閉じる")
              }
            }
            Spacer()
            Menu {
              ForEach(PaneTab.Kind.allCases, id: \.self) { kind in
                Button(kind.title) { model.editLayout { $0.open(kind, in: id) } }
              }
            } label: {
              Image(systemName: "plus")
            }.help("開く")
          }
          .padding(UIStyle.spacing)
          .dropDestination(for: String.self) { values, _ in
            guard let tab = values.first, model.layout.tab(tab) != nil else { return false }
            model.editLayout { $0.move(tab, to: id) }
            return true
          }
          Divider()
          if let tab = tabs.first(where: { $0.id == active }) {
            ContentUnavailableView(tab.kind.title, systemImage: "rectangle")
          } else {
            Color.clear
          }
        }
        .dropDestination(for: String.self) { values, position in
          guard let tab = values.first, model.layout.tab(tab) != nil else { return false }
          let distances: [(PaneLayout.Edge, CGFloat)] = [
            (.left, position.x), (.right, geometry.size.width - position.x), (.top, position.y),
            (.bottom, geometry.size.height - position.y),
          ]
          let edge = distances.min { $0.1 < $1.1 }?.0
          model.editLayout { $0.move(tab, to: id, edge: edge) }
          return true
        }
      }.frame(minWidth: UIStyle.minimumPaneSize, minHeight: UIStyle.minimumPaneSize)
    }
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
      let fraction = draggingRatio ?? ratio
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
            DragGesture().onChanged { drag in
              guard length > 0 else { return }
              let delta = horizontal ? drag.translation.width : drag.translation.height
              draggingRatio = min(0.95, max(0.05, ratio + delta / length))
            }.onEnded { _ in
              if let value = draggingRatio { model.editLayout { $0.resize(id, ratio: value) } }
              draggingRatio = nil
            }
          )
          .accessibilityLabel("分割のサイズ")
          .accessibilityAdjustableAction { direction in
            let value = min(0.95, max(0.05, ratio + (direction == .increment ? 0.05 : -0.05)))
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
